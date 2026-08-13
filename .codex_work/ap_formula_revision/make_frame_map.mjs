import fs from "node:fs/promises";
import path from "node:path";

const workDir = "/Users/thomm15/Documents/Stats_Paper_AUPRC_extension/.codex_work/ap_formula_revision";
const layoutsDir = path.join(workDir, "template-inspect", "layouts");

async function sourceAid(sourceSlide, shapeId) {
  const layoutPath = path.join(
    layoutsDir,
    `source-slide-${String(sourceSlide).padStart(2, "0")}.layout.json`,
  );
  const layout = JSON.parse(await fs.readFile(layoutPath, "utf8"));
  const element = layout.elements.find((candidate) => String(candidate.id) === String(shapeId));
  if (!element?.aid) {
    throw new Error(`No source aid for slide ${sourceSlide}, shape ${shapeId}`);
  }
  return element.aid;
}

const preserve = (outputSlide, sourceSlide) => ({
  outputSlide,
  sourceSlide,
  narrativeRole: `preserve-only source slide ${sourceSlide}`,
  reuseMode: "duplicate-slide",
  editTargets: [],
});

const outputSlides = [
  preserve(1, 1),
  preserve(2, 2),
  {
    outputSlide: 3,
    sourceSlide: 3,
    narrativeRole: "clarify the discrete AP components",
    reuseMode: "duplicate-slide",
    editTargets: [
      { action: "rewrite", sourceElementId: await sourceAid(3, "2"), reason: "connect precision and recall to the AP sum" },
      { action: "rewrite", sourceElementId: await sourceAid(3, "7"), reason: "define P_n" },
      { action: "rewrite", sourceElementId: await sourceAid(3, "8"), reason: "define precision at step n" },
      { action: "rewrite", sourceElementId: await sourceAid(3, "11"), reason: "define R_n" },
      { action: "rewrite", sourceElementId: await sourceAid(3, "12"), reason: "define recall at step n" },
      { action: "rewrite", sourceElementId: await sourceAid(3, "15"), reason: "state the contribution of each AP step" },
    ],
  },
  {
    outputSlide: 4,
    sourceSlide: 35,
    narrativeRole: "introduce the AP integral and its discrete threshold sum",
    reuseMode: "duplicate-slide",
    editTargets: [
      { action: "rewrite", sourceElementId: await sourceAid(35, "14"), reason: "move the slide into the WHY AP section" },
      { action: "rewrite", sourceElementId: await sourceAid(35, "2"), reason: "state the integral-to-sum relationship" },
      { action: "rewrite", sourceElementId: await sourceAid(35, "4"), reason: "move the slide into the WHY AP section" },
      { action: "rewrite", sourceElementId: await sourceAid(35, "5"), reason: "assign the inserted slide number" },
      { action: "replace", sourceElementId: await sourceAid(35, "15"), reason: "show the continuous and discrete AP formulas" },
      { action: "replace", sourceElementId: await sourceAid(35, "16"), reason: "show the recall increment term" },
      { action: "rewrite", sourceElementId: await sourceAid(35, "9"), reason: "define the recall increment" },
      { action: "replace", sourceElementId: await sourceAid(35, "17"), reason: "show the precision term" },
      { action: "rewrite", sourceElementId: await sourceAid(35, "11"), reason: "define precision at the nth threshold" },
      { action: "replace", sourceElementId: await sourceAid(35, "18"), reason: "show one weighted AP contribution" },
      { action: "rewrite", sourceElementId: await sourceAid(35, "13"), reason: "explain how the discrete contributions add to AP" },
    ],
  },
];

for (let outputSlide = 5; outputSlide <= 41; outputSlide += 1) {
  const sourceSlide = outputSlide - 1;
  outputSlides.push({
    outputSlide,
    sourceSlide,
    narrativeRole: `renumber preserved source slide ${sourceSlide}`,
    reuseMode: "duplicate-slide",
    editTargets: [
      { action: "rewrite", sourceElementId: await sourceAid(sourceSlide, "5"), reason: "shift the visible slide number after insertion" },
    ],
  });
}

const frameMap = {
  outputSlides,
  omittedSourceSlides: [],
};

await fs.writeFile(
  path.join(workDir, "template-frame-map.json"),
  `${JSON.stringify(frameMap, null, 2)}\n`,
);
