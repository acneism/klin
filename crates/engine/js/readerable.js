(() => {
  if (!(document.documentElement instanceof HTMLHtmlElement) || document.contentType === "application/pdf") {
    return false;
  }
  const isNodeVisible = (node) => node.clientHeight > 0 && node.clientWidth > 0;
  return isProbablyReaderable(document, isNodeVisible);
})()
