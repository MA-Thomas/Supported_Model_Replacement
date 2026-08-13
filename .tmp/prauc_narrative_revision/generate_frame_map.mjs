import fs from "node:fs/promises";
import path from "node:path";

const workspace = "/Users/thomm15/Documents/Stats_Paper_AUPRC_extension/.tmp/prauc_narrative_revision";
const layoutsDir = path.join(workspace, "template-inspect", "layouts");

const editedNames = new Map();
for (let slide = 2; slide <= 31; slide += 1) {
  editedNames.set(slide, new Map([[`title-${slide}`, "rewrite"]]));
}
editedNames.set(9, new Map([
  ["title-9", "rewrite"],
  ["num-9-0", "rewrite-and-reposition"],
  ["head-9-0", "rewrite"],
  ["body-9-0", "rewrite"],
  ["num-9-1", "rewrite-and-reposition"],
  ["head-9-1", "rewrite"],
  ["body-9-1", "rewrite"],
  ["num-9-2", "rewrite-and-reposition"],
  ["head-9-2", "rewrite"],
  ["body-9-2", "rewrite"],
  ["bottom-9", "rewrite"],
]));
editedNames.get(17).set("lead-17", "rewrite");
editedNames.get(20).set("why-20", "rewrite");
editedNames.get(23).set("meaning-23", "rewrite");
editedNames.get(23).set("why-23", "rewrite");
editedNames.get(28).set("num-28-0", "rewrite");
editedNames.get(28).set("num-28-1", "rewrite");
editedNames.get(28).set("num-28-2", "rewrite");
for (const slide of [39, 40, 46]) {
  editedNames.set(slide, new Map([[`title-${slide}`, "rewrite"]]));
}

const roles = {
  1: "brand title preserve-only",
  2: "situation",
  3: "criterion",
  4: "tension",
  5: "criterion",
  6: "boundary",
  7: "resolution",
  8: "consequence",
  9: "tension and regime comparison",
  10: "criterion",
  11: "tension",
  12: "consequence",
  13: "resolution",
  14: "tension and comparison",
  15: "diagnostic consequence",
  16: "consequence",
  17: "estimation tension",
  18: "limitation",
  19: "calibration tension",
  20: "resolution",
  21: "replacement tension",
  22: "secondary tension",
  23: "resolution",
  24: "resolution",
  25: "boundary",
  26: "consequence",
  27: "interpretation question",
  28: "reporting criterion",
  29: "limitation",
  30: "scope boundary",
  31: "synthesis",
  32: "section divider preserve-only",
  39: "technical tension",
  40: "technical tension",
  46: "computational tension",
};

const outputSlides = [];
for (let slide = 1; slide <= 50; slide += 1) {
  const layoutPath = path.join(layoutsDir, `source-slide-${String(slide).padStart(2, "0")}.layout.json`);
  const layout = JSON.parse(await fs.readFile(layoutPath, "utf8"));
  const byName = new Map(layout.elements.map((element) => [element.name, element]));
  const targets = [];
  const edits = editedNames.get(slide);
  if (edits) {
    for (const [name, action] of edits.entries()) {
      const element = byName.get(name);
      if (!element?.aid) throw new Error(`Missing ${name} on slide ${slide}`);
      targets.push({ action, shapeId: element.aid });
    }
  } else if (![1, 32].includes(slide)) {
    const titleName = slide === 1 ? "deck-title" : slide === 32 ? "divider-title-32" : `title-${slide}`;
    const title = byName.get(titleName);
    if (title?.aid) targets.push({ action: "keep", shapeId: title.aid });
  }
  outputSlides.push({
    outputSlide: slide,
    sourceSlide: slide,
    narrativeRole: roles[slide] ?? "reference content",
    reuseMode: "duplicate-slide",
    editTargets: targets,
  });
}

const frameMap = { outputSlides, omittedSourceSlides: [] };
await fs.writeFile(
  path.join(workspace, "template-frame-map.json"),
  `${JSON.stringify(frameMap, null, 2)}\n`,
  "utf8",
);
