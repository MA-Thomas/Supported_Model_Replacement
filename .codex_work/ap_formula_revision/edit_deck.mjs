import fs from "node:fs/promises";
import path from "node:path";
import { FileBlob, PresentationFile } from "@oai/artifact-tool";

const workDir = "/Users/thomm15/Documents/Stats_Paper_AUPRC_extension/.codex_work/ap_formula_revision";
const starterPath = path.join(workDir, "template-starter.pptx");
const finalPath =
  "/Users/thomm15/Documents/Stats_Paper_AUPRC_extension/Stats_Paper_Extending_PRAUC_v13_Lab_Walkthrough_AP_formula.pptx";
const assetDir = path.join(workDir, "formula-assets");
const finalRenderDir = path.join(workDir, "final-render");
const finalLayoutDir = path.join(workDir, "final-layout");

await fs.mkdir(finalRenderDir, { recursive: true });
await fs.mkdir(finalLayoutDir, { recursive: true });

const presentation = await PresentationFile.importPptx(await FileBlob.load(starterPath));
const snapshot = await presentation.inspect({
  kind: "slide,textbox,shape,image,notes",
  include: "id,slide,name,text,textPreview,bbox",
  maxChars: 400000,
});
const records = snapshot.ndjson
  .split("\n")
  .filter((line) => line.trim().startsWith("{"))
  .map((line) => JSON.parse(line));

function findRecord(slide, kind, predicate, label) {
  const record = records.find(
    (candidate) => candidate.slide === slide && candidate.kind === kind && predicate(candidate),
  );
  if (!record) throw new Error(`Could not find ${label} on slide ${slide}`);
  return record;
}

function rewriteText(slide, name, oldText, newText) {
  const record = findRecord(
    slide,
    "textbox",
    (candidate) => candidate.name === name && candidate.text === oldText,
    `${name} with text ${JSON.stringify(oldText)}`,
  );
  presentation.resolve(record.id).text.replace(oldText, newText);
}

function setStyledText(slide, name, oldText, newText, style) {
  const record = findRecord(
    slide,
    "textbox",
    (candidate) => candidate.name === name && candidate.text === oldText,
    `${name} with text ${JSON.stringify(oldText)}`,
  );
  const shape = presentation.resolve(record.id);
  shape.text = newText;
  shape.text.style = style;
}

async function replaceImage(slide, name, fileName, alt) {
  const record = findRecord(
    slide,
    "image",
    (candidate) => candidate.name === name,
    `image ${name}`,
  );
  const image = presentation.resolve(record.id);
  const oldFrame = image.frame;
  const oldGeometry = image.geometry;
  const oldBorderRadius = image.borderRadius;
  const oldRotation = image.rotation;
  const oldFlipHorizontal = image.flipHorizontal;
  const oldFlipVertical = image.flipVertical;
  const oldLockAspectRatio = image.lockAspectRatio;
  const bytes = await fs.readFile(path.join(assetDir, fileName));
  image.replace({
    blob: bytes,
    contentType: "image/png",
    alt,
    fit: "contain",
  });
  image.frame = oldFrame;
  image.crop = { left: 0, top: 0, right: 0, bottom: 0 };
  image.geometry = oldGeometry;
  image.borderRadius = oldBorderRadius;
  image.rotation = oldRotation;
  image.flipHorizontal = oldFlipHorizontal;
  image.flipVertical = oldFlipVertical;
  image.lockAspectRatio = oldLockAspectRatio;
}

setStyledText(
  3,
  "Rectangle 1",
  "Average Precision depends on precision (averaged over recall)",
  "AP combines precision with each gain in recall",
  { fontSize: 48, bold: true, color: "#11253D", alignment: "left", typeface: "Calibri" },
);
rewriteText(3, "Rectangle 6", "PRECISION", "PRECISION AT STEP n");
setStyledText(
  3,
  "Rectangle 7",
  "Of the cases we selected,\nhow many are positive?",
  "P_n: precision after\nthreshold step n",
  { fontSize: 30, bold: true, color: "#11253D", alignment: "left", typeface: "Calibri" },
);
rewriteText(3, "Rectangle 10", "RECALL", "RECALL AT STEP n");
setStyledText(
  3,
  "Rectangle 11",
  "Of all positive cases,\nhow many did we select?",
  "R_n: recall after\nthreshold step n",
  { fontSize: 30, bold: true, color: "#11253D", alignment: "left", typeface: "Calibri" },
);
rewriteText(
  3,
  "Rectangle 14",
  "Move the score threshold → both quantities change together.",
  "AP adds (R_n − R_(n−1)) × P_n at each step.",
);

setStyledText(
  4,
  "Rectangle 13",
  "BACKUP",
  "WHY AP?",
  {
    fontSize: 15,
    bold: true,
    color: "#2B6CB0",
    alignment: "left",
    verticalAlignment: "middle",
    insets: { top: 0, right: 9.6, bottom: 0, left: 9.6 },
    typeface: "Calibri",
  },
);
setStyledText(
  4,
  "Rectangle 1",
  "Empirical prior-standardized AP is\na weighted threshold sum",
  "AP is the area under\nthe precision–recall curve",
  { fontSize: 48, bold: true, color: "#11253D", alignment: "left", typeface: "Calibri" },
);
setStyledText(
  4,
  "Rectangle 3",
  "BACKUP",
  "WHY AP?",
  {
    fontSize: 14,
    bold: true,
    color: "#5F6B78",
    alignment: "left",
    verticalAlignment: "middle",
    insets: { top: 0, right: 9.6, bottom: 0, left: 9.6 },
    typeface: "Calibri",
  },
);
rewriteText(4, "Rectangle 4", "35", "04");
rewriteText(
  4,
  "Rectangle 8",
  "recall gained when a score block enters",
  "recall gained between adjacent thresholds",
);
rewriteText(
  4,
  "Rectangle 10",
  "target-population positive mass selected",
  "precision after threshold n",
);
rewriteText(
  4,
  "Rectangle 12",
  "target-population negative mass selected",
  "one weighted contribution to AP",
);
await replaceImage(
  4,
  "Picture 19",
  "ap-formulas.png",
  "Average Precision integral and discrete weighted recall-increment sum",
);
await replaceImage(
  4,
  "Picture 20",
  "recall-increment.png",
  "Recall increment R sub n minus R sub n minus one",
);
await replaceImage(
  4,
  "Picture 22",
  "precision-term.png",
  "Precision P sub n",
);
await replaceImage(
  4,
  "Picture 24",
  "weighted-term.png",
  "Weighted Average Precision contribution",
);

for (let slideNumber = 5; slideNumber <= 41; slideNumber += 1) {
  const record = findRecord(
    slideNumber,
    "textbox",
    (candidate) =>
      candidate.name === "Rectangle 4" &&
      typeof candidate.text === "string" &&
      /^\d{2}$/.test(candidate.text) &&
      candidate.bbox?.[0] >= 1150,
    "visible slide number",
  );
  const shape = presentation.resolve(record.id);
  shape.text.replace(record.text, String(slideNumber).padStart(2, "0"));
}

presentation.slides.getItem(2).speakerNotes.textFrame.setText(
  [
    "Define the two axes, then connect them to the empirical sum: P_n is precision after step n and R_n − R_(n−1) is the recall gain. Each product is one contribution to AP.",
    "",
    "[Sources]",
    "- Stats_Paper_Extending_PRAUC_v12.tex.",
  ].join("\n"),
);
presentation.slides.getItem(3).speakerNotes.textFrame.setText(
  [
    "Present AP first as the area under the precision–recall curve, then as the empirical sum over threshold steps. Emphasize that the sum weights precision by recall gained.",
    "",
    "[Sources]",
    "- User-supplied AP formula.",
    "- Stats_Paper_Extending_PRAUC_v12.tex.",
  ].join("\n"),
);

for (const [index, slide] of presentation.slides.items.entries()) {
  const stem = `slide-${String(index + 1).padStart(2, "0")}`;
  const png = await presentation.export({ slide, format: "png", scale: 1 });
  await fs.writeFile(path.join(finalRenderDir, `${stem}.png`), new Uint8Array(await png.arrayBuffer()));
  const layout = await slide.export({ format: "layout" });
  await fs.writeFile(path.join(finalLayoutDir, `${stem}.layout.json`), await layout.text());
}

const montage = await presentation.export({
  format: "webp",
  montage: true,
  scale: 1,
});
await fs.writeFile(
  path.join(workDir, "final-montage.webp"),
  new Uint8Array(await montage.arrayBuffer()),
);

const finalInspect = await presentation.inspect({
  kind: "slide,textbox,shape,image,chart,table,notes,layout",
  maxChars: 400000,
});
await fs.writeFile(path.join(workDir, "final-inspect.ndjson"), finalInspect.ndjson);

const pptx = await PresentationFile.exportPptx(presentation);
await pptx.save(finalPath);
console.log(finalPath);
