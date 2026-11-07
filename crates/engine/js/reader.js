(() => {
  const CLASSES_TO_PRESERVE = [
    "caption",
    "emoji",
    "hidden",
    "invisible",
    "sr-only",
    "visually-hidden",
    "visuallyhidden",
    "wp-caption",
    "wp-caption-text",
    "wp-smiley",
  ];
  const doc = document.cloneNode(true);
  const article = new Readability(doc, { classesToPreserve: CLASSES_TO_PRESERVE }).parse();
  if (!article) {
    return "null";
  }
  return JSON.stringify({
    title: article.title || "",
    byline: article.byline || null,
    content: article.content || "",
    lang: article.lang || null,
    dir: article.dir || null,
    length: article.length || 0,
  });
})()
