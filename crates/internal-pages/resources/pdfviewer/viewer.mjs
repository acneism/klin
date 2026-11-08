const RESOURCES = "klin-res://pdfjs/";
const pdfjs = await import(`${RESOURCES}build/pdf.min.mjs`);
const { EventBus, PDFLinkService, PDFViewer } = await import(`${RESOURCES}web/pdf_viewer.min.mjs`);

pdfjs.GlobalWorkerOptions.workerSrc = `${RESOURCES}build/pdf.worker.min.mjs`;

const MIN_SCALE = 0.1;
const MAX_SCALE = 10;
const ZOOM_STEP = 1.1;

const $ = (id) => document.getElementById(id);
const container = $("viewerContainer");
const pageNumber = $("pageNumber");
const scaleSelect = $("scaleSelect");
const customScale = $("customScale");

const fileName = () => {
  const path = new URL(location.href).pathname;
  const name = path.slice(path.lastIndexOf("/") + 1);
  try {
    return decodeURIComponent(name) || location.href;
  } catch {
    return name || location.href;
  }
};
document.title = fileName();

const eventBus = new EventBus();
const linkService = new PDFLinkService({ eventBus, externalLinkTarget: 0 });
const viewer = new PDFViewer({ container, eventBus, linkService });
linkService.setViewer(viewer);

const showError = (text) => {
  const message = $("message");
  message.textContent = text;
  message.hidden = false;
};

const updateButtons = () => {
  const page = viewer.currentPageNumber;
  const count = viewer.pagesCount;
  $("previous").disabled = page <= 1;
  $("next").disabled = page >= count;
  $("zoomOut").disabled = viewer.currentScale <= MIN_SCALE;
  $("zoomIn").disabled = viewer.currentScale >= MAX_SCALE;
};

eventBus.on("pagesinit", () => {
  viewer.currentScaleValue = "auto";
  for (const id of ["pageNumber", "scaleSelect", "zoomIn", "zoomOut"]) {
    $(id).disabled = false;
  }
  pageNumber.max = viewer.pagesCount;
  $("numPages").textContent = `из ${viewer.pagesCount}`;
  updateButtons();
});

eventBus.on("pagechanging", ({ pageNumber: page }) => {
  pageNumber.value = page;
  updateButtons();
});

eventBus.on("scalechanging", ({ scale, presetValue }) => {
  const preset = presetValue && [...scaleSelect.options].find((o) => o.value === String(presetValue));
  if (preset && preset !== customScale) {
    customScale.hidden = true;
    scaleSelect.value = preset.value;
  } else {
    customScale.textContent = `${Math.round(scale * 100)}%`;
    customScale.hidden = false;
    scaleSelect.value = "custom";
  }
  updateButtons();
});

$("previous").addEventListener("click", () => viewer.previousPage());
$("next").addEventListener("click", () => viewer.nextPage());
pageNumber.addEventListener("change", () => {
  const page = Math.trunc(Number(pageNumber.value));
  if (page >= 1 && page <= viewer.pagesCount) {
    viewer.currentPageNumber = page;
  } else {
    pageNumber.value = viewer.currentPageNumber;
  }
});
scaleSelect.addEventListener("change", () => {
  if (scaleSelect.value !== "custom") {
    viewer.currentScaleValue = scaleSelect.value;
  }
});
const zoom = (factor) => {
  const scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, viewer.currentScale * factor));
  viewer.currentScale = Math.round(scale * 100) / 100;
};
$("zoomIn").addEventListener("click", () => zoom(ZOOM_STEP));
$("zoomOut").addEventListener("click", () => zoom(1 / ZOOM_STEP));

window.addEventListener("keydown", (event) => {
  if (event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement) {
    return;
  }
  const plain = !event.ctrlKey && !event.altKey && !event.metaKey;
  if (plain && (event.key === "n" || event.key === "j")) {
    viewer.nextPage();
  } else if (plain && (event.key === "p" || event.key === "k")) {
    viewer.previousPage();
  } else if (plain && event.key === "Home") {
    viewer.currentPageNumber = 1;
  } else if (plain && event.key === "End") {
    viewer.currentPageNumber = viewer.pagesCount;
  } else {
    return;
  }
  event.preventDefault();
});

try {
  const task = pdfjs.getDocument({
    url: location.href,
    disableFontFace: true,
    isEvalSupported: false,
    enableXfa: false,
    cMapUrl: `${RESOURCES}cmaps/`,
    cMapPacked: true,
    standardFontDataUrl: `${RESOURCES}standard_fonts/`,
    wasmUrl: `${RESOURCES}wasm/`,
  });
  const pdf = await task.promise;
  viewer.setDocument(pdf);
  linkService.setDocument(pdf, null);
  const { info, metadata } = await pdf.getMetadata().catch(() => ({}));
  let title = info?.Title;
  const metadataTitle = metadata?.get("dc:title");
  if (metadataTitle && metadataTitle !== "Untitled" && !/[\uFFF0-\uFFFF]/g.test(metadataTitle)) {
    title = metadataTitle;
  }
  if (title && title.trim()) {
    document.title = title.trim();
  }
  container.focus();
} catch (error) {
  console.error(error);
  showError(
    error?.name === "PasswordException"
      ? "Этот PDF-документ защищён паролем, такие документы пока не поддерживаются."
      : "При загрузке PDF-документа произошла ошибка.",
  );
}
