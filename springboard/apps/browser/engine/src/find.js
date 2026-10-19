function findInPage(mode, query) {
  var selection = window.getSelection();
  if (mode === 'c' || !query || !document.body) {
    selection.removeAllRanges();
    window.__bifrostFind = null;
    return 0;
  }
  var walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {
    acceptNode: function (node) {
      var parent = node.parentElement;
      if (!parent || parent.closest('script,style,noscript,template')) return NodeFilter.FILTER_REJECT;
      var style = getComputedStyle(parent);
      if (style.visibility === 'hidden' || style.visibility === 'collapse') return NodeFilter.FILTER_REJECT;
      var range = document.createRange();
      range.selectNodeContents(node);
      return range.getClientRects().length ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT;
    }
  });
  var nodes = [], text = '', node;
  // Bound browser-owned search bookkeeping even on very large pages.
  while (nodes.length < 50000 && text.length < 2000000 && (node = walker.nextNode())) {
    nodes.push({ node: node, start: text.length });
    text += node.data;
  }
  var escaped = typeof RegExp.escape === 'function' ? RegExp.escape(query) : query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  var expression = new RegExp(escaped, 'giu'), matches = [], match;
  while (matches.length < 10000 && (match = expression.exec(text))) matches.push([match.index, match.index + match[0].length]);
  if (!matches.length) { selection.removeAllRanges(); window.__bifrostFind = null; return 0; }
  var previous = window.__bifrostFind;
  var index = mode === 'n' || !previous || previous.query !== query || !Number.isInteger(previous.index)
    ? 0 : (previous.index + (mode === 'b' ? -1 : 1) + matches.length) % matches.length;
  window.__bifrostFind = { query: query, index: index };
  function position(offset) {
    for (var i = 0; i < nodes.length; i++) {
      var item = nodes[i];
      if (offset <= item.start + item.node.length) return [item.node, offset - item.start];
    }
    return null;
  }
  var start = position(matches[index][0]), end = position(matches[index][1]);
  if (start && end) {
    var range = document.createRange();
    range.setStart(start[0], start[1]);
    range.setEnd(end[0], end[1]);
    selection.removeAllRanges();
    selection.addRange(range);
    start[0].parentElement.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }
  return matches.length;
}
