"""Semantic features for the parity matrix, on Titan's embedding and rerank services (hamr-server).

Endpoints come from PARITY_EMBED_URL / PARITY_RERANK_URL, else AKURAI_EMBED_URL / AKURAI_RERANK_URL, else
http://127.0.0.1:8084 (OpenAI-style /v1/embeddings, and /v1/rerank). Vectors are cached by text hash in
~/.cache/bifrost-parity/embeddings-<model>.npz, so only new or changed rows are embedded again.

  search QUERY   embed the query, take the nearest feature rows by cosine, rerank them, print the best
  related        nearest frameworks for each framework (by the mean of its row vectors) -> related.json
  themes         cluster Gap rows across frameworks into work themes (k-means) -> themes.json
"""
import hashlib
import json
import os
import urllib.request

import numpy as np

BASE = os.environ.get("PARITY_EMBED_URL") or os.environ.get("AKURAI_EMBED_URL") or "http://127.0.0.1:8084"
RERANK = os.environ.get("PARITY_RERANK_URL") or os.environ.get("AKURAI_RERANK_URL") or "http://127.0.0.1:8084"
CACHE_DIR = os.path.join(os.path.expanduser("~"), ".cache", "bifrost-parity")
BATCH = 64


def _post(url, body):
    req = urllib.request.Request(url, data=json.dumps(body).encode(), headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.load(r)


def _hash(text):
    return hashlib.sha256(text.encode()).hexdigest()[:24]


class Embedder:
    """Embeds texts through the service, with an on-disk cache keyed by model and text hash."""

    def __init__(self):
        probe = _post(BASE + "/v1/embeddings", {"input": ["probe"]})
        self.model = probe.get("model", "unknown")
        os.makedirs(CACHE_DIR, exist_ok=True)
        self.path = os.path.join(CACHE_DIR, f"embeddings-{self.model}.npz")
        self.cache = {}
        if os.path.exists(self.path):
            z = np.load(self.path)
            self.cache = dict(zip(z["keys"].tolist(), z["vecs"]))
        self.dirty = False

    def embed(self, texts):
        keys = [_hash(t) for t in texts]
        todo = [(k, t) for k, t in dict(zip(keys, texts)).items() if k not in self.cache]
        for i in range(0, len(todo), BATCH):
            chunk = todo[i:i + BATCH]
            data = _post(BASE + "/v1/embeddings", {"input": [t for _, t in chunk]})["data"]
            for (k, _), d in zip(chunk, sorted(data, key=lambda d: d["index"])):
                v = np.asarray(d["embedding"], dtype=np.float32)
                self.cache[k] = v / (np.linalg.norm(v) or 1.0)
            self.dirty = True
        if self.dirty:
            ks = list(self.cache)
            np.savez(self.path, keys=np.array(ks), vecs=np.stack([self.cache[k] for k in ks]))
            self.dirty = False
        return np.stack([self.cache[k] for k in keys]) if keys else np.zeros((0, 1), dtype=np.float32)


def rerank(query, docs):
    """Rerank scores for docs against query, in the order of docs."""
    out = _post(RERANK + "/v1/rerank", {"query": query, "documents": docs})
    scores = [0.0] * len(docs)
    for r in out.get("results", []):
        scores[r["index"]] = r["relevance_score"]
    return scores, out.get("model", "unknown")


def row_text(e, r):
    """The text a feature row is embedded as: framework, feature, reference API, BifrOSt side and note."""
    return (f"{e['name']}: {r.get('feature', '')}. Reference API: {r.get('apple', '')}. "
            f"BifrOSt: {r.get('bifrost', '')}. {r.get('note', '')}").strip()


def all_rows(entries):
    return [(e, r) for e in entries for r in e["rows"]]


def search(entries, query, top=10, pool=60, status=None):
    rows = [(e, r) for e, r in all_rows(entries) if not status or r["status"] in status]
    emb = Embedder()
    m = emb.embed([row_text(e, r) for e, r in rows])
    q = emb.embed([query])[0]
    sims = m @ q
    idx = np.argsort(-sims)[:pool]
    cand = [rows[i] for i in idx]
    scores, model = rerank(query, [row_text(e, r) for e, r in cand])
    order = sorted(range(len(cand)), key=lambda i: -scores[i])[:top]
    return [(scores[i], float(sims[idx[i]]), cand[i][0], cand[i][1]) for i in order], emb.model, model


def related(entries, k=5):
    emb = Embedder()
    vecs = {}
    for e in entries:
        texts = [row_text(e, r) for r in e["rows"]] or [f"{e['name']}: {e['summary']}"]
        v = emb.embed(texts).mean(axis=0)
        vecs[e["slug"]] = v / (np.linalg.norm(v) or 1.0)
    slugs = list(vecs)
    m = np.stack([vecs[s] for s in slugs])
    sims = m @ m.T
    out = {}
    for i, s in enumerate(slugs):
        order = [j for j in np.argsort(-sims[i]) if j != i][:k]
        out[s] = [{"slug": slugs[j], "score": round(float(sims[i, j]), 3)} for j in order]
    return out, emb.model


def _kmeans(x, k, iters=40, seed=7):
    rng = np.random.default_rng(seed)
    # k-means++ seeding on cosine distance (rows are unit vectors)
    centers = [x[rng.integers(len(x))]]
    for _ in range(1, k):
        d = 1 - np.max(x @ np.stack(centers).T, axis=1)
        p = np.clip(d, 0, None) ** 2
        centers.append(x[rng.choice(len(x), p=p / p.sum())])
    c = np.stack(centers)
    labels = np.zeros(len(x), dtype=int)
    for _ in range(iters):
        new = np.argmax(x @ c.T, axis=1)
        if np.array_equal(new, labels) and _:
            break
        labels = new
        for j in range(k):
            pts = x[labels == j]
            if len(pts):
                v = pts.mean(axis=0)
                c[j] = v / (np.linalg.norm(v) or 1.0)
    return labels, c


def themes(entries, k=None, status=("gap",)):
    rows = [(e, r) for e, r in all_rows(entries) if r["status"] in status]
    if not rows:
        return [], None
    emb = Embedder()
    x = emb.embed([row_text(e, r) for e, r in rows])
    k = k or max(8, min(60, round(len(rows) ** 0.5 * 1.2)))
    labels, centers = _kmeans(x, k)
    out = []
    for j in range(k):
        members = [i for i in range(len(rows)) if labels[i] == j]
        if not members:
            continue
        sims = x[members] @ centers[j]
        ranked = [members[i] for i in np.argsort(-sims)]
        medoid = rows[ranked[0]]
        fws = {}
        for i in ranked:
            fws.setdefault(rows[i][0]["slug"], rows[i][0]["name"])
        out.append({
            "id": f"theme-{len(out) + 1:02d}",
            "title": medoid[1].get("feature", ""),
            "size": len(members),
            "frameworks": [{"slug": s, "name": n} for s, n in fws.items()],
            "rows": [{"slug": rows[i][0]["slug"], "framework": rows[i][0]["name"], "feature": rows[i][1].get("feature", ""),
                      "apple": rows[i][1].get("apple", ""), "cohesion": round(float(x[i] @ centers[j]), 3)} for i in ranked[:25]],
        })
    out.sort(key=lambda t: -t["size"])
    for n, t in enumerate(out, 1):
        t["id"] = f"theme-{n:02d}"
    return out, emb.model
