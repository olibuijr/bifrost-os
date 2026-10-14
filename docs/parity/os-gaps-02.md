# Cache, link, swap and ambient models

This branch adds four pure models: a document-to-document link/backlink index, an LRU per-app cache purge plan that
skips running apps, a swap growth rule, and timestamped ambient-light samples. The caller still persists and updates
the link index, removes caches, creates swap files, and reads sensor channels. The link and swap rules do not have a
direct row in the reference framework inventory; neither is a search-term index or a memory-pressure service.

Headless tests cover replacement and backlink lookup, purge ordering and shortfall, swap thresholds and caps, and
sensor conversion/validation. The CacheDelete and ambient sample rows remain partial pending shell wiring and Midget
checks.
