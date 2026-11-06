((query, caseSensitive, wholeWord, direction) => {
  const KEY = Symbol.for("klin.find");
  const selection = window.getSelection();
  if (direction === "clear" || query === "") {
    window[KEY] = undefined;
    if (selection) selection.removeAllRanges();
    return JSON.stringify({ current: 0, total: 0, wrapped: null });
  }

  const SKIP = new Set(["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "TEXTAREA", "SELECT", "OPTION"]);
  const hidden = new Map();
  const isHidden = (el) => {
    if (hidden.has(el)) return hidden.get(el);
    let result = false;
    if (SKIP.has(el.tagName)) result = true;
    else {
      const style = window.getComputedStyle(el);
      result = style.display === "none" || style.visibility === "hidden";
      if (!result && el.parentElement) result = isHidden(el.parentElement);
    }
    hidden.set(el, result);
    return result;
  };

  const collect = () => {
    const root = document.body || document.documentElement;
    if (!root) return [];
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    const segments = [];
    let text = "";
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const parent = node.parentElement;
      if (!parent || isHidden(parent) || !node.data) continue;
      segments.push({ node, start: text.length });
      text += node.data;
    }
    const norm = (s) => (caseSensitive ? s : s.toLowerCase());
    const haystack = norm(text);
    const needle = norm(query);

    const sameLength = haystack.length === text.length;
    const source = sameLength ? haystack : text;
    const wordChar = /[\p{L}\p{N}_]/u;
    const matches = [];
    for (let i = source.indexOf(needle); i !== -1 && needle; i = source.indexOf(needle, i + 1)) {
      if (wholeWord) {
        const before = i > 0 ? source[i - 1] : "";
        const after = source[i + needle.length] || "";
        if ((before && wordChar.test(before)) || (after && wordChar.test(after))) continue;
      }
      matches.push([i, i + needle.length]);
    }
    const locate = (pos) => {
      let lo = 0, hi = segments.length - 1;
      while (lo < hi) {
        const mid = (lo + hi + 1) >> 1;
        if (segments[mid].start <= pos) lo = mid; else hi = mid - 1;
      }
      const s = segments[lo];
      return [s.node, pos - s.start];
    };
    return matches.map(([a, b]) => {
      const [sn, so] = locate(a);
      const [en, eo] = locate(b - 1);
      return { sn, so, en, eo: eo + 1 };
    });
  };

  const signature = `${query}\u0000${caseSensitive}\u0000${wholeWord}`;
  let state = window[KEY];
  const stale = !state || state.signature !== signature ||
    state.matches.some((m) => !m.sn.isConnected || !m.en.isConnected);
  if (direction === "first" || stale) {
    state = { signature, matches: collect(), index: -1 };
    window[KEY] = state;
  }
  const total = state.matches.length;
  if (total === 0) {
    if (selection) selection.removeAllRanges();
    return JSON.stringify({ current: 0, total: 0, wrapped: null });
  }
  let wrapped = null;
  if (direction === "previous") {
    state.index -= 1;
    if (state.index < 0) { state.index = total - 1; wrapped = "bottom"; }
  } else {
    const before = state.index;
    state.index = direction === "first" ? 0 : state.index + 1;
    if (state.index >= total) { state.index = 0; wrapped = "top"; }
    if (direction === "first" && before < 0) wrapped = null;
  }
  const m = state.matches[state.index];
  const range = document.createRange();
  range.setStart(m.sn, m.so);
  range.setEnd(m.en, m.eo);
  if (selection) {
    selection.removeAllRanges();
    selection.addRange(range);
  }

  const el = m.sn.parentElement;
  if (el) {
    const r = el.getBoundingClientRect();
    if (r.height > 0 || r.width > 0) {
      const visible = r.top >= 0 && r.bottom <= window.innerHeight;
      if (!visible) window.scrollBy(0, r.top + r.height / 2 - window.innerHeight / 2);
    } else {
      el.scrollIntoView();
    }
  }
  return JSON.stringify({ current: state.index + 1, total, wrapped });
})
