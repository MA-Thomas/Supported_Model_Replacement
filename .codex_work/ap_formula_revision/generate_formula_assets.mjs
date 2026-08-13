import fs from "node:fs/promises";
import path from "node:path";
import sharp from "/Users/thomm15/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/sharp/lib/index.js";

const workDir = "/Users/thomm15/Documents/Stats_Paper_AUPRC_extension/.codex_work/ap_formula_revision";
const assetDir = path.join(workDir, "formula-assets");
await fs.mkdir(assetDir, { recursive: true });

async function renderMarkupPng(name, width, height, lines, fontSize, options = {}) {
  const lineHeight = options.lineHeight ?? Math.round(fontSize * 1.2);
  const totalHeight = lineHeight * lines.length;
  const startY = (height - totalHeight) / 2 + fontSize * 0.86;
  const text = lines
    .map(
      (line, index) =>
        `<text x="50%" y="${startY + index * lineHeight}" text-anchor="middle" ` +
        `font-family="Cambria Math, STIX Two Math, Times New Roman, serif" ` +
        `font-size="${fontSize}" font-style="${options.italic ? "italic" : "normal"}" ` +
        `fill="#11253D">${line}</text>`,
    )
    .join("");
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" ` +
    `viewBox="0 0 ${width} ${height}">${text}</svg>`;
  await sharp(Buffer.from(svg)).png().toFile(path.join(assetDir, `${name}.png`));
}

const sub = (value, size = 35) =>
  `<tspan baseline-shift="sub" font-size="${size}">${value}</tspan>`;
const sup = (value, size = 35) =>
  `<tspan baseline-shift="super" font-size="${size}">${value}</tspan>`;

await renderMarkupPng(
  "ap-formulas",
  1382,
  184,
  [
    `AP = ∫${sub("0")}${sup("1")} Precision d(Recall)`,
    `AP = Σ${sub("n")} (R${sub("n")} − R${sub("n−1")}) P${sub("n")}`,
  ],
  58,
  { lineHeight: 82 },
);
await renderMarkupPng(
  "recall-increment",
  435,
  96,
  [`R${sub("n", 34)} − R${sub("n−1", 34)}`],
  56,
  { italic: true },
);
await renderMarkupPng(
  "precision-term",
  187,
  96,
  [`P${sub("n", 36)}`],
  60,
  { italic: true },
);
await renderMarkupPng(
  "weighted-term",
  392,
  96,
  [`(R${sub("n", 29)} − R${sub("n−1", 29)})P${sub("n", 29)}`],
  48,
  { italic: true },
);
