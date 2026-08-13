import fs from "node:fs/promises";
import { Presentation, PresentationFile } from "@oai/artifact-tool";

const W = 1280;
const H = 720;
const BG = "#F7F5F0";
const INK = "#11253D";
const MUTED = "#5F6B78";
const FAINT = "#D9DEE5";
const BLUE = "#2B6CB0";
const BLUE_LIGHT = "#DCEAF7";
const TEAL = "#0E7C66";
const TEAL_LIGHT = "#D9EEE8";
const ORANGE = "#D97706";
const ORANGE_LIGHT = "#F7E7CF";
const PLUM = "#6B4E8E";
const PLUM_LIGHT = "#E8E0F0";
const RED = "#B5473C";
const RED_LIGHT = "#F4DEDB";
const WHITE = "#FFFFFF";
const FONT = "Arial";
const BUILD = "/Users/thomm15/Documents/Stats_Paper_AUPRC_extension/.deck_build_v12";
const OUT = `${BUILD}/deck_with_equation_fallbacks.pptx`;
const EQUATION_DIR = `${BUILD}/equations`;

const EQUATION_IDS = [
  "s03_precision", "s03_recall", "s04_ap_mean", "s04_ap_result",
  "s05_rank1", "s05_rank3", "s05_rank6", "s06_rates",
  "s07_r", "s07_f", "s07_pi", "s08_precision", "s09_invariance",
  "s10_chance", "s10_cnap", "s13_assessment", "s17_support",
  "s18_support2", "s22_decomposition", "s24_order1", "s24_order2",
  "s25_calibration", "s26_bad_subtraction", "s27_a", "s27_b",
  "s27_bias", "s28_ap1", "s28_ap2", "s28_ap3", "s29_pair",
  "s29_population", "s29_replication", "s29_expectation",
  "s31_constraint", "s31_left", "s31_right", "s35_ap",
  "s35_recall", "s35_positive", "s35_negative", "s37_estimator",
  "s38_cost",
];

const equationAssets = Object.fromEntries(
  await Promise.all(
    EQUATION_IDS.map(async (id) => [id, await fs.readFile(`${EQUATION_DIR}/${id}.png`)]),
  ),
);

const presentation = Presentation.create({
  slideSize: { width: W, height: H },
});

function shape(slide, geometry, left, top, width, height, fill = "none", lineFill = "none", lineWidth = 0, radius) {
  return slide.shapes.add({
    geometry,
    position: { left, top, width, height },
    fill,
    line: { style: "solid", fill: lineFill, width: lineWidth },
    ...(radius ? { borderRadius: radius } : {}),
  });
}

function text(slide, value, left, top, width, height, opts = {}) {
  const box = shape(slide, "textbox", left, top, width, height);
  box.text = value;
  box.text.style = {
    fontFamily: FONT,
    fontSize: opts.size ?? 26,
    bold: opts.bold ?? false,
    italic: opts.italic ?? false,
    color: opts.color ?? INK,
    alignment: opts.align ?? "left",
  };
  return box;
}

function richText(slide, runs, left, top, width, height, opts = {}) {
  const box = shape(slide, "textbox", left, top, width, height);
  box.text = runs.map((r) => ({
    run: r.text,
    textStyle: {
      fontFamily: FONT,
      fontSize: r.size ?? opts.size ?? 26,
      bold: r.bold ?? false,
      italic: r.italic ?? false,
      color: r.color ?? opts.color ?? INK,
    },
  }));
  box.text.style = { alignment: opts.align ?? "left" };
  return box;
}

function equation(slide, id, left, top, width, height, alt) {
  return slide.images.add({
    blob: equationAssets[id],
    contentType: "image/png",
    alt: alt ?? `Mathematical equation: ${id}`,
    fit: "contain",
    position: { left, top, width, height },
  });
}

function rule(slide, left, top, width, color = FAINT, height = 3) {
  return shape(slide, "rect", left, top, width, height, color);
}

function addFooter(slide, n, section) {
  text(slide, section.toUpperCase(), 72, 680, 760, 20, { size: 14, bold: true, color: MUTED });
  text(slide, String(n).padStart(2, "0"), 1160, 678, 48, 22, { size: 15, bold: true, color: MUTED, align: "right" });
}

function addTitle(slide, title, section, n, accent = BLUE, titleSize = 48) {
  slide.background.fill = BG;
  text(slide, section.toUpperCase(), 72, 34, 460, 20, { size: 15, bold: true, color: accent });
  const lineCount = title.includes("\n") ? 2 : 1;
  text(slide, title, 72, 70, 1136, lineCount === 2 ? 108 : 64, {
    size: titleSize,
    bold: true,
    color: INK,
  });
  rule(slide, 72, lineCount === 2 ? 181 : 142, 1136, FAINT, 2);
  addFooter(slide, n, section);
}

function addNotes(slide, talk, source) {
  slide.speakerNotes.textFrame.setText(
    `${talk}\n\n[Sources]\n- ${source}`,
  );
}

function newSlide(title, section, n, accent = BLUE, source = "Stats_Paper_Extending_PRAUC_v12.tex", titleSize = 48) {
  const slide = presentation.slides.add();
  addTitle(slide, title, section, n, accent, titleSize);
  addNotes(slide, "", source);
  return slide;
}

function callout(slide, value, left, top, width, height, fill, color = INK, size = 26, bold = true) {
  const box = shape(slide, "roundRect", left, top, width, height, fill, "none", 0, "rounded-xl");
  text(slide, value, left + 20, top + 18, width - 40, height - 30, { size, bold, color, align: "center" });
  return box;
}

function smallLabel(slide, value, left, top, width, color = MUTED) {
  return text(slide, value.toUpperCase(), left, top, width, 22, { size: 15, bold: true, color });
}

function dot(slide, x, y, color, label = "") {
  shape(slide, "ellipse", x, y, 38, 38, color);
  if (label) text(slide, label, x, y + 5, 38, 24, { size: 21, bold: true, color: WHITE, align: "center" });
}

function arrowText(slide, x, y, size = 32, color = MUTED) {
  text(slide, "→", x, y, 50, 42, { size, bold: true, color, align: "center" });
}

function bar(slide, x, y, width, height, value, max, fill, label, valueLabel) {
  shape(slide, "roundRect", x, y, width, height, "#E8EBEF", "none", 0, "rounded-xl");
  shape(slide, "roundRect", x, y, Math.max(8, width * value / max), height, fill, "none", 0, "rounded-xl");
  text(slide, label, x, y - 32, width, 24, { size: 20, bold: true, color: INK });
  text(slide, valueLabel, x + width + 16, y + 4, 90, 28, { size: 22, bold: true, color: fill });
}

function addLineChart(slide, left, top, width, height, categories, series, opts = {}) {
  return slide.charts.add("line", {
    position: { left, top, width, height },
    categories,
    series,
    hasLegend: opts.legend ?? true,
    legend: {
      position: "bottom",
      overlay: false,
      textStyle: { fontSize: 16, fill: INK },
    },
    lineOptions: { grouping: "standard", smooth: opts.smooth ?? false },
    xAxis: {
      visible: true,
      majorGridlines: { style: "solid", fill: "#E5E7EB", width: 1 },
      textStyle: { fontSize: 15, fill: MUTED },
    },
    yAxis: {
      visible: true,
      minimumScale: opts.yMin ?? 0,
      maximumScale: opts.yMax ?? 1,
      majorUnit: opts.major ?? 0.2,
      majorGridlines: { style: "solid", fill: "#D9DEE5", width: 1 },
      textStyle: { fontSize: 15, fill: MUTED },
    },
    chartFill: BG,
    plotAreaFill: BG,
    chartLine: { style: "solid", fill: "none", width: 0 },
    plotAreaLine: { style: "solid", fill: "none", width: 0 },
  });
}

function standardAP(labels, pi) {
  const nPos = labels.filter((x) => x === 1).length;
  const nNeg = labels.length - nPos;
  let tp = 0;
  let fp = 0;
  let prevRecall = 0;
  let ap = 0;
  for (const y of labels) {
    if (y === 1) tp += 1;
    else fp += 1;
    const recall = tp / nPos;
    if (y === 1) {
      const fpr = fp / nNeg;
      const precision = (pi * recall) / (pi * recall + (1 - pi) * fpr);
      ap += (recall - prevRecall) * precision;
      prevRecall = recall;
    }
  }
  return ap;
}

function cnAP(labels, pi) {
  const ap = standardAP(labels, pi);
  return (ap - pi) / (1 - pi);
}

// 01 — title
{
  const slide = presentation.slides.add();
  slide.background.fill = BG;
  smallLabel(slide, "LAB WALKTHROUGH", 76, 54, 360, BLUE);
  text(slide, "What did the performance\nclaim survive?", 76, 126, 850, 182, { size: 62, bold: true, color: INK });
  text(slide, "A pedagogical route from average precision to prevalence-robust,\nreplication-supported model replacement", 80, 352, 840, 82, { size: 27, color: MUTED });
  shape(slide, "roundRect", 952, 126, 170, 170, BLUE_LIGHT, "none", 0, "rounded-xl");
  text(slide, "AP", 968, 160, 138, 60, { size: 58, bold: true, color: BLUE, align: "center" });
  text(slide, "↓", 1010, 226, 56, 40, { size: 32, bold: true, color: ORANGE, align: "center" });
  shape(slide, "roundRect", 1018, 292, 170, 170, TEAL_LIGHT, "none", 0, "rounded-xl");
  text(slide, "claim", 1034, 336, 138, 50, { size: 37, bold: true, color: TEAL, align: "center" });
  text(slide, "survived", 1034, 390, 138, 34, { size: 23, color: TEAL, align: "center" });
  rule(slide, 76, 604, 1112, FAINT, 2);
  text(slide, "Stats_Paper_Extending_PRAUC_v12", 76, 628, 700, 24, { size: 18, color: MUTED });
  text(slide, "Computational lab presentation", 890, 628, 298, 24, { size: 18, color: MUTED, align: "right" });
  addNotes(slide,
    "Open with the paper’s governing question. The talk is not about proposing another context-free score; it is about making the conditions of a performance claim explicit.",
    "Stats_Paper_Extending_PRAUC_v12.tex, abstract and Section 1.");
}

// 02
{
  const slide = newSlide("AUROC describes separation—but our decisions\nreceive a selected set", "Why AP?", 2, BLUE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 1.");
  smallLabel(slide, "SEPARATION", 92, 220, 280, BLUE);
  for (let i = 0; i < 8; i++) {
    dot(slide, 92 + i * 55, 270 + (i % 2) * 6, i < 4 ? BLUE : ORANGE);
  }
  text(slide, "How often does a positive\noutrank a negative?", 92, 344, 430, 72, { size: 28, bold: true });
  arrowText(slide, 573, 302, 46, MUTED);
  smallLabel(slide, "SELECTED SET", 690, 220, 300, TEAL);
  shape(slide, "roundRect", 690, 260, 470, 150, TEAL_LIGHT, "none", 0, "rounded-xl");
  for (let i = 0; i < 7; i++) dot(slide, 724 + i * 56, 310, i < 3 ? BLUE : ORANGE);
  text(slide, "Who actually enters the review,\nassay, or intervention queue?", 690, 444, 470, 72, { size: 28, bold: true, color: TEAL });
  callout(slide, "The composition of the selected set is itself a scientific outcome.", 250, 552, 780, 72, WHITE, INK, 25, true);
}

// 03
{
  const slide = newSlide("Precision asks whom we selected;\nrecall asks whom we missed", "Why AP?", 3, BLUE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 1 and Appendix A.");
  shape(slide, "roundRect", 92, 222, 488, 252, BLUE_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "PRECISION", 122, 250, 200, BLUE);
  text(slide, "Of the cases we selected,\nhow many are positive?", 122, 300, 410, 78, { size: 30, bold: true });
  equation(slide, "s03_precision", 130, 398, 394, 54, "Precision equals true positives divided by true positives plus false positives.");
  shape(slide, "roundRect", 700, 222, 488, 252, TEAL_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "RECALL", 730, 250, 200, TEAL);
  text(slide, "Of all positive cases,\nhow many did we select?", 730, 300, 410, 78, { size: 30, bold: true });
  equation(slide, "s03_recall", 738, 398, 394, 54, "Recall equals true positives divided by true positives plus false negatives.");
  callout(slide, "Move the score threshold → both quantities change together.", 304, 530, 672, 68, WHITE, INK, 24, true);
}

// 04
{
  const slide = newSlide("A PR curve records what happens\nas the selected set grows", "Why AP?", 4, BLUE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 1 and Appendix A.");
  const labels = [1, 0, 1, 0, 0, 1, 0, 0];
  smallLabel(slide, "ONE RANKED LIST", 80, 220, 260, BLUE);
  labels.forEach((y, i) => {
    shape(slide, "roundRect", 80 + i * 65, 270, 54, 72, y ? BLUE : ORANGE_LIGHT, y ? BLUE : ORANGE, 2, "rounded-xl");
    text(slide, String(i + 1), 80 + i * 65, 282, 54, 20, { size: 15, bold: true, color: MUTED, align: "center" });
    text(slide, y ? "+" : "−", 80 + i * 65, 306, 54, 28, { size: 25, bold: true, color: y ? WHITE : ORANGE, align: "center" });
  });
  arrowText(slide, 610, 280, 50, MUTED);
  addLineChart(slide, 700, 216, 470, 340,
    ["0", ".33", ".33", ".67", ".67", ".67", "1.0", "1.0"],
    [{
      name: "precision",
      values: [100, 100, 50, 66.7, 50, 40, 50, 37.5],
      line: { style: "solid", fill: BLUE, width: 4 },
      marker: { symbol: "circle", size: 7 },
    }],
    { legend: false, yMin: 0, yMax: 100, major: 20 });
  text(slide, "recall →", 884, 556, 110, 24, { size: 17, bold: true, color: MUTED, align: "center" });
  text(slide, "Every positive expands recall; every false positive can dilute precision.", 166, 546, 448, 58, { size: 25, bold: true, color: INK });
}

// 05
{
  const slide = newSlide("AP rises when positives\nappear early", "Why AP?", 5, BLUE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Appendix A.", 46);
  smallLabel(slide, "POSITIVE RANKS", 92, 224, 240, BLUE);
  const vals = [
    { rank: "rank 1", equation: "s05_rank1" },
    { rank: "rank 3", equation: "s05_rank3" },
    { rank: "rank 6", equation: "s05_rank6" },
  ];
  vals.forEach((d, i) => {
    const y = 278 + i * 86;
    dot(slide, 100, y, BLUE, "+");
    text(slide, d.rank, 164, y + 3, 130, 30, { size: 24, bold: true });
    equation(slide, d.equation, 300, y - 4, 238, 46, `Precision at ${d.rank}.`);
  });
  shape(slide, "roundRect", 650, 250, 500, 238, WHITE, FAINT, 2, "rounded-xl");
  equation(slide, "s04_ap_mean", 682, 286, 436, 82, "Average precision is the mean of the precision values at positive ranks.");
  equation(slide, "s04_ap_result", 748, 380, 304, 62, "Average precision equals 0.72.");
  callout(slide, "AP is a weighted summary of precision over recall—not the area of a smooth geometric shape.", 180, 548, 920, 72, BLUE_LIGHT, INK, 24, true);
}

// 06
{
  const slide = newSlide("The ranking can stay unchanged\nwhile precision collapses", "Population", 6, ORANGE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 1 numerical example.");
  smallLabel(slide, "SAME OPERATING BEHAVIOR", 78, 218, 320, MUTED);
  equation(slide, "s06_rates", 72, 246, 470, 56, "True-positive rate is 80 percent and false-positive rate is 5 percent.");
  bar(slide, 98, 366, 430, 44, 0.94, 1, BLUE, "50% prevalence", "0.94");
  bar(slide, 98, 488, 430, 44, 0.14, 1, ORANGE, "1% prevalence", "0.14");
  shape(slide, "roundRect", 710, 240, 430, 296, ORANGE_LIGHT, "none", 0, "rounded-xl");
  text(slide, "The classifier did not change.", 752, 292, 346, 42, { size: 30, bold: true, color: INK, align: "center" });
  text(slide, "The supply of negatives did.", 752, 372, 346, 42, { size: 30, bold: true, color: ORANGE, align: "center" });
  text(slide, "Precision therefore describes\na ranking in a population.", 752, 448, 346, 62, { size: 25, color: INK, align: "center" });
}

// 07
{
  const slide = newSlide("Change the class prior—but preserve\nwithin-class ranking behavior", "Population", 7, PLUM,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 2.1.");
  smallLabel(slide, "KEEP", 102, 224, 150, TEAL);
  shape(slide, "roundRect", 90, 264, 420, 248, TEAL_LIGHT, "none", 0, "rounded-xl");
  equation(slide, "s07_r", 112, 298, 376, 62, "Recall at threshold c.");
  equation(slide, "s07_f", 112, 378, 376, 62, "False-positive rate at threshold c.");
  text(slide, "Observed class-conditional\nranking behavior", 122, 454, 356, 52, { size: 22, color: MUTED, align: "center" });
  arrowText(slide, 594, 352, 52, MUTED);
  smallLabel(slide, "CHANGE", 758, 224, 180, ORANGE);
  shape(slide, "roundRect", 744, 264, 420, 248, ORANGE_LIGHT, "none", 0, "rounded-xl");
  equation(slide, "s07_pi", 788, 322, 332, 70, "Target prevalence equals the probability of a positive outcome.");
  text(slide, "The target population’s\nclass prior", 786, 414, 336, 58, { size: 25, color: MUTED, align: "center" });
}

// 08
{
  const slide = newSlide("Prior-standardized precision makes\nthe target prevalence explicit", "Population", 8, TEAL,
    "Stats_Paper_Extending_PRAUC_v12.tex, Equation (1).");
  shape(slide, "roundRect", 92, 230, 1096, 116, WHITE, FAINT, 2, "rounded-xl");
  equation(slide, "s08_precision", 142, 246, 996, 86, "Prior-standardized precision at threshold c.");
  const pis = ["1%", "5%", "10%", "20%", "50%"];
  const vals = [0.139, 0.457, 0.64, 0.8, 0.941];
  pis.forEach((p, i) => {
    const x = 118 + i * 210;
    text(slide, `${p} target`, x, 404, 150, 28, { size: 21, bold: true, color: MUTED, align: "center" });
    shape(slide, "roundRect", x, 450, 150, 74, i === 0 ? ORANGE_LIGHT : BLUE_LIGHT, "none", 0, "rounded-xl");
    text(slide, vals[i].toFixed(2), x, 466, 150, 38, { size: 30, bold: true, color: i === 0 ? ORANGE : BLUE, align: "center" });
  });
  text(slide, "The example fixes recall at 0.80 and the false-positive rate at 0.05.", 278, 566, 724, 28, { size: 21, color: MUTED, align: "center" });
}

// 09
{
  const slide = newSlide("Reweighting is licensed only when\nclass-conditional scores transport", "Population", 9, ORANGE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 2.2 and Equation (3).");
  shape(slide, "roundRect", 84, 226, 520, 302, TEAL_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "LICENSED", 116, 254, 180, TEAL);
  equation(slide, "s09_invariance", 112, 304, 464, 116, "Observed and target class-conditional score distributions are equal.");
  text(slide, "Only the class prior changes.", 116, 466, 456, 32, { size: 23, color: TEAL, align: "center" });
  shape(slide, "roundRect", 676, 226, 520, 302, RED_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "NOT LICENSED", 708, 254, 220, RED);
  text(slide, "Severity shifts\nMeasurement shifts\nWithin-class scores shift", 730, 310, 412, 116, { size: 28, bold: true, color: INK, align: "center" });
  text(slide, "Those changes belong in the regime.", 708, 466, 456, 32, { size: 23, color: RED, align: "center" });
  callout(slide, "Invariance is a prespecified transport claim—not something the reweighted curve can prove.", 178, 568, 924, 64, WHITE, INK, 23, true);
}

// 10
{
  const slide = newSlide("AP needs a common baseline\nbefore prevalences can be compared", "Population", 10, PLUM,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 2.3 and Equation (4).");
  text(slide, "Random ranking has", 106, 248, 330, 38, { size: 28, color: MUTED });
  equation(slide, "s10_chance", 102, 294, 330, 66, "Under random ranking, average precision equals target prevalence.");
  text(slide, "So a raw AP of 0.30 means\nsomething different at 1%\nthan at 20% prevalence.", 106, 390, 430, 110, { size: 26, bold: true, color: INK });
  arrowText(slide, 566, 350, 50, MUTED);
  shape(slide, "roundRect", 676, 240, 478, 240, WHITE, FAINT, 2, "rounded-xl");
  equation(slide, "s10_cnap", 710, 278, 410, 82, "Chance-normalized average precision at target prevalence pi.");
  text(slide, "0 marks population random ranking", 710, 378, 410, 30, { size: 22, color: MUTED, align: "center" });
  text(slide, "1 marks perfect ranking", 710, 420, 410, 30, { size: 22, color: MUTED, align: "center" });
  callout(slide, "Normalize after evaluating the ranking in the target population.", 326, 546, 628, 68, PLUM_LIGHT, INK, 24, true);
}

// 11
{
  const slide = newSlide("Chance normalization aligns the endpoints—\nnot the whole prevalence profile", "Population", 11, BLUE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 2.3; values computed from the slide 4 ranking.");
  const labels = [1, 0, 1, 0, 0, 1, 0, 0];
  const pis = [0.01, 0.03, 0.05, 0.1, 0.2, 0.4, 0.6];
  const cats = pis.map((p) => p.toFixed(2));
  const apVals = pis.map((p) => Number((100 * standardAP(labels, p)).toFixed(1)));
  const cnVals = pis.map((p) => Number((100 * cnAP(labels, p)).toFixed(1)));
  addLineChart(slide, 76, 218, 770, 376, cats, [
    { name: "AP at target prevalence", values: apVals, line: { style: "solid", fill: BLUE, width: 4 }, marker: { symbol: "circle", size: 6 } },
    { name: "CNAP at target prevalence", values: cnVals, line: { style: "solid", fill: TEAL, width: 4 }, marker: { symbol: "diamond", size: 6 } },
  ], { legend: true, yMin: 0, yMax: 100, major: 20 });
  shape(slide, "roundRect", 884, 250, 300, 270, WHITE, FAINT, 2, "rounded-xl");
  text(slide, "Same ranking", 916, 282, 236, 34, { size: 25, bold: true, color: INK, align: "center" });
  text(slide, "Same endpoints", 916, 348, 236, 34, { size: 25, bold: true, color: TEAL, align: "center" });
  text(slide, "Difficulty varies\nby prevalence", 916, 406, 236, 62, { size: 26, bold: true, color: ORANGE, align: "center" });
  text(slide, "target prevalence →", 326, 596, 270, 22, { size: 17, bold: true, color: MUTED, align: "center" });
}

// 12
{
  const slide = newSlide("If the claim must hold everywhere,\nan average answers the wrong question", "Population", 12, ORANGE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 2.4 and Equation (6).");
  const vals = [0.55, 0.48, 0.31, 0.52];
  const ps = [".01", ".05", ".20", ".40"];
  vals.forEach((v, i) => {
    const x = 94 + i * 210;
    shape(slide, "roundRect", x, 268, 160, 126, i === 2 ? ORANGE_LIGHT : BLUE_LIGHT, "none", 0, "rounded-xl");
    text(slide, `${ps[i]} target`, x, 284, 160, 26, { size: 19, bold: true, color: MUTED, align: "center" });
    text(slide, v.toFixed(2), x, 326, 160, 44, { size: 34, bold: true, color: i === 2 ? ORANGE : BLUE, align: "center" });
  });
  arrowText(slide, 950, 314, 42, MUTED);
  shape(slide, "roundRect", 1020, 268, 170, 126, TEAL_LIGHT, "none", 0, "rounded-xl");
  text(slide, "minimum", 1020, 286, 170, 26, { size: 20, bold: true, color: MUTED, align: "center" });
  text(slide, "0.31", 1020, 326, 170, 44, { size: 34, bold: true, color: TEAL, align: "center" });
  text(slide, "Average: 0.47", 214, 456, 300, 38, { size: 28, bold: true, color: MUTED, align: "center" });
  text(slide, "but the claim failed at 0.31", 666, 456, 400, 38, { size: 28, bold: true, color: ORANGE, align: "center" });
  callout(slide, "Robust CNAP is the largest level attained at every declared prevalence.", 226, 550, 828, 68, TEAL_LIGHT, INK, 24, true);
}

// 13
{
  const slide = newSlide("A performance claim needs three declarations:\npopulations, regime, and attempts", "Assessment", 13, PLUM,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 1.");
  equation(slide, "s13_assessment", 340, 230, 600, 78, "The assessment is the triple of population set, replication regime, and attempt count.");
  const items = [
    { x: 100, fill: BLUE_LIGHT, color: BLUE, top: "Π", body: "Where must the\nclaim hold?" },
    { x: 470, fill: PLUM_LIGHT, color: PLUM, top: "ℛ", body: "What may vary in a\ncomplete replication?" },
    { x: 840, fill: ORANGE_LIGHT, color: ORANGE, top: "K", body: "How many attempts\nmust be survived?" },
  ];
  items.forEach((d) => {
    shape(slide, "roundRect", d.x, 350, 330, 202, d.fill, "none", 0, "rounded-xl");
    text(slide, d.top, d.x, 376, 330, 54, { size: 42, bold: true, color: d.color, align: "center" });
    text(slide, d.body, d.x + 28, 446, 274, 66, { size: 24, bold: true, color: INK, align: "center" });
  });
}

// 14
{
  const slide = newSlide("“Replication” is incomplete until we say\nwhat is allowed to change", "Assessment", 14, PLUM,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 3.1 and Table 1.");
  const rows = [
    ["Fixed model", "new units", "same environment", "same fitted model"],
    ["New environment", "new units", "new environment", "same fitted model"],
    ["Repeated development", "new units", "new environment", "refit + reselect"],
  ];
  smallLabel(slide, "REGIME", 86, 214, 250, PLUM);
  text(slide, "EVALUATION", 414, 214, 200, 22, { size: 15, bold: true, color: MUTED, align: "center" });
  text(slide, "ENVIRONMENT", 680, 214, 200, 22, { size: 15, bold: true, color: MUTED, align: "center" });
  text(slide, "MODEL", 952, 214, 200, 22, { size: 15, bold: true, color: MUTED, align: "center" });
  rows.forEach((r, i) => {
    const y = 260 + i * 104;
    const fill = i === 0 ? BLUE_LIGHT : i === 1 ? PLUM_LIGHT : ORANGE_LIGHT;
    const color = i === 0 ? BLUE : i === 1 ? PLUM : ORANGE;
    shape(slide, "roundRect", 80, y, 1110, 78, fill, "none", 0, "rounded-xl");
    text(slide, r[0], 100, y + 22, 260, 30, { size: 22, bold: true, color });
    text(slide, r[1], 398, y + 22, 220, 30, { size: 21, color: INK, align: "center" });
    text(slide, r[2], 650, y + 22, 260, 30, { size: 21, color: INK, align: "center" });
    text(slide, r[3], 918, y + 22, 240, 30, { size: 21, color: INK, align: "center" });
  });
  text(slide, "A unit bootstrap identifies only the first target.", 346, 594, 588, 34, { size: 25, bold: true, color: RED, align: "center" });
}

// 15
{
  const slide = newSlide("Prevalence is computed within each draw;\nother conditions are generated by the regime", "Assessment", 15, BLUE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 3.2.");
  shape(slide, "roundRect", 84, 234, 494, 290, BLUE_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "COMPUTED CHALLENGE", 116, 262, 250, BLUE);
  text(slide, "Evaluate every target prevalence\nwithin the same replication", 116, 326, 430, 78, { size: 29, bold: true, color: INK, align: "center" });
  text(slide, "Licensed by score-level\nprior-shift invariance", 116, 442, 430, 56, { size: 22, color: MUTED, align: "center" });
  shape(slide, "roundRect", 700, 234, 494, 290, PLUM_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "GENERATED CHALLENGE", 732, 262, 270, PLUM);
  text(slide, "Draw environments, units,\nor development runs from the regime", 732, 326, 430, 78, { size: 29, bold: true, color: INK, align: "center" });
  text(slide, "Cannot be recovered by\nreweighting one evaluation", 732, 442, 430, 56, { size: 22, color: MUTED, align: "center" });
  callout(slide, "If invariance fails, prevalence becomes part of the environments generated by the regime.", 216, 558, 848, 72, WHITE, INK, 22, true);
}

// 16
{
  const slide = newSlide("Expected performance can conceal a replication\nthat defeated the claim", "Corroboration", 16, ORANGE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 4.1.");
  shape(slide, "roundRect", 104, 246, 360, 182, TEAL_LIGHT, "none", 0, "rounded-xl");
  text(slide, "Replication 1", 104, 272, 360, 28, { size: 21, bold: true, color: MUTED, align: "center" });
  text(slide, "0.70", 104, 322, 360, 60, { size: 53, bold: true, color: TEAL, align: "center" });
  text(slide, "+", 494, 310, 58, 50, { size: 40, bold: true, color: MUTED, align: "center" });
  shape(slide, "roundRect", 584, 246, 360, 182, RED_LIGHT, "none", 0, "rounded-xl");
  text(slide, "Replication 2", 584, 272, 360, 28, { size: 21, bold: true, color: MUTED, align: "center" });
  text(slide, "0.05", 584, 322, 360, 60, { size: 53, bold: true, color: RED, align: "center" });
  arrowText(slide, 974, 308, 42, MUTED);
  shape(slide, "roundRect", 1042, 246, 150, 182, ORANGE_LIGHT, "none", 0, "rounded-xl");
  text(slide, "mean", 1042, 274, 150, 28, { size: 20, bold: true, color: MUTED, align: "center" });
  text(slide, "0.375", 1042, 330, 150, 46, { size: 31, bold: true, color: ORANGE, align: "center" });
  text(slide, "A claim at 0.375 did not survive replication 2.", 256, 506, 768, 44, { size: 31, bold: true, color: RED, align: "center" });
}

// 17
{
  const slide = newSlide("Support is the expected level retained\nby every declared challenge", "Corroboration", 17, TEAL,
    "Stats_Paper_Extending_PRAUC_v12.tex, Equation (7).");
  const vals = [0.62, 0.47, 0.71, 0.29];
  vals.forEach((v, i) => {
    const x = 112 + i * 220;
    text(slide, `draw ${i + 1}`, x, 228, 150, 28, { size: 20, bold: true, color: MUTED, align: "center" });
    shape(slide, "roundRect", x, 274, 150, 108, i === 3 ? ORANGE_LIGHT : BLUE_LIGHT, "none", 0, "rounded-xl");
    text(slide, v.toFixed(2), x, 304, 150, 44, { size: 34, bold: true, color: i === 3 ? ORANGE : BLUE, align: "center" });
  });
  arrowText(slide, 1008, 300, 42, MUTED);
  shape(slide, "roundRect", 1080, 274, 120, 108, TEAL_LIGHT, "none", 0, "rounded-xl");
  text(slide, "min", 1080, 290, 120, 24, { size: 18, bold: true, color: MUTED, align: "center" });
  text(slide, "0.29", 1080, 324, 120, 38, { size: 30, bold: true, color: TEAL, align: "center" });
  shape(slide, "roundRect", 256, 458, 768, 98, WHITE, FAINT, 2, "rounded-xl");
  equation(slide, "s17_support", 290, 472, 700, 72, "Support is the regime expectation of the minimum over K replication outcomes.");
  text(slide, "Expectation is over repeated executions of the declared regime.", 304, 584, 672, 28, { size: 21, color: MUTED, align: "center" });
}

// 18
{
  const slide = newSlide("With two attempts, corroboration charges\nhalf the replication disagreement", "Corroboration", 18, TEAL,
    "Stats_Paper_Extending_PRAUC_v12.tex, Equation (8).");
  shape(slide, "roundRect", 106, 238, 1068, 112, WHITE, FAINT, 2, "rounded-xl");
  equation(slide, "s18_support2", 150, 252, 980, 82, "For two attempts, support equals expected performance minus half the expected absolute replication difference.");
  shape(slide, "roundRect", 130, 416, 390, 134, BLUE_LIGHT, "none", 0, "rounded-xl");
  text(slide, "Expected performance", 160, 444, 330, 30, { size: 24, bold: true, color: BLUE, align: "center" });
  text(slide, "0.50", 160, 488, 330, 40, { size: 34, bold: true, color: INK, align: "center" });
  text(slide, "−", 578, 454, 64, 48, { size: 42, bold: true, color: MUTED, align: "center" });
  shape(slide, "roundRect", 700, 416, 450, 134, ORANGE_LIGHT, "none", 0, "rounded-xl");
  text(slide, "Half the expected disagreement", 730, 444, 390, 30, { size: 24, bold: true, color: ORANGE, align: "center" });
  text(slide, "0.12", 730, 488, 390, 40, { size: 34, bold: true, color: INK, align: "center" });
  callout(slide, "Supported level: 0.38", 450, 580, 380, 62, TEAL_LIGHT, TEAL, 27, true);
}

// 19
{
  const slide = newSlide("In Popper’s sense, survival is support—\nnot probability of truth", "Corroboration", 19, PLUM,
    "Stats_Paper_Extending_PRAUC_v12.tex, Sections 4.1–4.2; Popper (1959, 1963).");
  shape(slide, "roundRect", 92, 232, 504, 296, PLUM_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "WHAT THE CLAIM EARNS", 124, 262, 260, PLUM);
  text(slide, "It survived specified\nopportunities for defeat.", 124, 332, 440, 90, { size: 32, bold: true, color: INK, align: "center" });
  text(slide, "The assessment makes those\nopportunities criticizable.", 124, 454, 440, 56, { size: 23, color: MUTED, align: "center" });
  shape(slide, "roundRect", 684, 232, 504, 296, RED_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "WHAT IT DOES NOT EARN", 716, 262, 300, RED);
  text(slide, "A probability that the\nmodel or claim is true.", 716, 332, 440, 90, { size: 32, bold: true, color: INK, align: "center" });
  text(slide, "Nor a fixed confidence level.", 716, 466, 440, 34, { size: 23, color: MUTED, align: "center" });
  text(slide, "Corroboration remains negative: the claim has not yet been defeated under 𝒜.", 192, 574, 896, 36, { size: 26, bold: true, color: PLUM, align: "center" });
}

// 20
{
  const slide = newSlide("A bootstrap estimates projected support;\nit does not manufacture replications", "Corroboration", 20, ORANGE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 4.2.");
  smallLabel(slide, "COMPUTATION", 92, 220, 220, BLUE);
  shape(slide, "roundRect", 92, 262, 432, 238, BLUE_LIGHT, "none", 0, "rounded-xl");
  text(slide, "1 observed evaluation", 122, 294, 372, 34, { size: 27, bold: true, color: INK, align: "center" });
  text(slide, "↓", 280, 338, 56, 38, { size: 30, bold: true, color: BLUE, align: "center" });
  text(slide, "1,000 bootstrap draws", 122, 386, 372, 34, { size: 27, bold: true, color: BLUE, align: "center" });
  text(slide, "Less Monte Carlo error", 122, 448, 372, 30, { size: 22, color: MUTED, align: "center" });
  smallLabel(slide, "EVIDENCE", 714, 220, 220, PLUM);
  shape(slide, "roundRect", 714, 262, 474, 238, PLUM_LIGHT, "none", 0, "rounded-xl");
  text(slide, "New sites, measurements,\nor development runs", 744, 306, 414, 70, { size: 29, bold: true, color: INK, align: "center" });
  text(slide, "Can reveal failures the bootstrap\nheld fixed.", 744, 414, 414, 58, { size: 23, color: PLUM, align: "center" });
  callout(slide, "More bootstrap draws improve computation; more attempts strengthen the claim; new environments change the evidence.", 170, 554, 940, 76, WHITE, INK, 22, true);
}

// 21
{
  const slide = newSlide("Each replication must face\nthe full prevalence set", "Corroboration", 21, ORANGE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 4.4 and Equation (12).");
  const cats = [".01", ".05", ".20", ".40"];
  addLineChart(slide, 74, 220, 740, 360, cats, [
    { name: "replication 1", values: [22, 38, 55, 61], line: { style: "solid", fill: BLUE, width: 4 }, marker: { symbol: "circle", size: 7 } },
    { name: "replication 2", values: [62, 54, 36, 18], line: { style: "solid", fill: ORANGE, width: 4 }, marker: { symbol: "diamond", size: 7 } },
  ], { legend: true, yMin: 0, yMax: 80, major: 20 });
  shape(slide, "roundRect", 856, 246, 328, 258, WHITE, FAINT, 2, "rounded-xl");
  text(slide, "Replication 1 fails first\nat low prevalence.", 884, 282, 272, 70, { size: 25, bold: true, color: BLUE, align: "center" });
  text(slide, "Replication 2 fails first\nat high prevalence.", 884, 392, 272, 70, { size: 25, bold: true, color: ORANGE, align: "center" });
  text(slide, "Each replication must reveal its own limiting population.", 190, 604, 900, 34, { size: 26, bold: true, color: INK, align: "center" });
}

// 22
{
  const slide = newSlide("Support equals mean performance\nminus two costs of challenge", "Corroboration", 22, TEAL,
    "Stats_Paper_Extending_PRAUC_v12.tex, Equation (13). Numerical values are illustrative.");
  const steps = [
    { x: 86, value: "0.54", label: "Best fixed-prevalence\nexpected profile", fill: BLUE_LIGHT, color: BLUE },
    { x: 364, value: "−0.08", label: "Movement of limiting\nprevalence", fill: ORANGE_LIGHT, color: ORANGE },
    { x: 642, value: "−0.06", label: "Replication\ndisagreement", fill: PLUM_LIGHT, color: PLUM },
    { x: 920, value: "0.40", label: "Supported\nperformance", fill: TEAL_LIGHT, color: TEAL },
  ];
  steps.forEach((d, i) => {
    shape(slide, "roundRect", d.x, 278, 224, 170, d.fill, "none", 0, "rounded-xl");
    text(slide, d.value, d.x, 306, 224, 48, { size: 39, bold: true, color: d.color, align: "center" });
    text(slide, d.label, d.x + 16, 374, 192, 58, { size: 21, bold: true, color: INK, align: "center" });
    if (i < steps.length - 1) arrowText(slide, d.x + 226, 330, 34, MUTED);
  });
  equation(slide, "s22_decomposition", 256, 500, 768, 72, "Support decomposes into the best fixed-prevalence expected profile minus prevalence movement and replication disagreement costs.");
  text(slide, "The costs diagnose why support is low; they are not new headline estimands.", 208, 586, 864, 32, { size: 23, color: MUTED, align: "center" });
}

// 23
{
  const slide = newSlide("A fixed-model bootstrap preserves\nthe order of the scientific claim", "Estimation", 23, BLUE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 5.1.");
  const steps = [
    { x: 80, num: "1", title: "Resample units", body: "Stratify by observed\noutcome class", fill: BLUE_LIGHT, color: BLUE },
    { x: 366, num: "2", title: "Build profile", body: "Evaluate every target\non those same units", fill: PLUM_LIGHT, color: PLUM },
    { x: 652, num: "3", title: "Retain worst target", body: "Keep the profile minimum\nfor each replication", fill: ORANGE_LIGHT, color: ORANGE },
    { x: 938, num: "4", title: "Apply support", body: "Average minima over\ngroups of attempts", fill: TEAL_LIGHT, color: TEAL },
  ];
  steps.forEach((d, i) => {
    shape(slide, "roundRect", d.x, 252, 246, 268, d.fill, "none", 0, "rounded-xl");
    shape(slide, "ellipse", d.x + 92, 274, 62, 62, d.color);
    text(slide, d.num, d.x + 92, 288, 62, 32, { size: 25, bold: true, color: WHITE, align: "center" });
    text(slide, d.title, d.x + 18, 358, 210, 36, { size: 24, bold: true, color: INK, align: "center" });
    text(slide, d.body, d.x + 18, 416, 210, 62, { size: 21, color: MUTED, align: "center" });
    if (i < steps.length - 1) arrowText(slide, d.x + 246, 354, 28, MUTED);
  });
  text(slide, "Resampling separately at each prevalence would destroy the joint profile.", 278, 576, 724, 32, { size: 24, bold: true, color: RED, align: "center" });
}

// 24
{
  const slide = newSlide("Finite-sample AP has a design anchor\nthat need not equal prevalence", "Design anchor", 24, ORANGE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 6.1.");
  smallLabel(slide, "ONE POSITIVE + ONE NEGATIVE; 50% PREVALENCE", 92, 216, 520, MUTED);
  shape(slide, "roundRect", 92, 266, 416, 190, BLUE_LIGHT, "none", 0, "rounded-xl");
  text(slide, "ordering 1", 92, 286, 416, 26, { size: 19, bold: true, color: MUTED, align: "center" });
  dot(slide, 220, 338, BLUE, "+");
  dot(slide, 290, 338, ORANGE, "−");
  equation(slide, "s24_order1", 116, 394, 368, 48, "For the favorable ordering, average precision and chance-normalized average precision both equal one.");
  shape(slide, "roundRect", 548, 266, 416, 190, ORANGE_LIGHT, "none", 0, "rounded-xl");
  text(slide, "ordering 2", 548, 286, 416, 26, { size: 19, bold: true, color: MUTED, align: "center" });
  dot(slide, 676, 338, ORANGE, "−");
  dot(slide, 746, 338, BLUE, "+");
  equation(slide, "s24_order2", 572, 384, 368, 68, "For the unfavorable ordering, average precision is one half and chance-normalized average precision is zero.");
  arrowText(slide, 988, 338, 40, MUTED);
  shape(slide, "roundRect", 1054, 290, 138, 146, RED_LIGHT, "none", 0, "rounded-xl");
  text(slide, "support", 1054, 310, 138, 30, { size: 22, bold: true, color: MUTED, align: "center" });
  text(slide, "0.25", 1054, 358, 138, 40, { size: 34, bold: true, color: RED, align: "center" });
  callout(slide, "Pure noise can return positive supported CNAP because stepwise AP rewards favorable finite orderings.", 166, 536, 948, 76, RED_LIGHT, INK, 23, true);
}

// 25
{
  const slide = newSlide("Conditional calibration makes one model\ninterpretable—but not comparable", "Design anchor", 25, PLUM,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 6.2 and Equations (17)–(18).");
  shape(slide, "roundRect", 82, 236, 510, 268, PLUM_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "FOR ONE FIXED SCORE VECTOR", 114, 266, 330, PLUM);
  text(slide, "Permute labels under the\ndesign’s allowed symmetry", 114, 322, 446, 70, { size: 28, bold: true, color: INK, align: "center" });
  text(slide, "→ estimate its conditional\nnull anchor", 114, 420, 446, 58, { size: 24, color: PLUM, align: "center" });
  arrowText(slide, 616, 338, 44, MUTED);
  shape(slide, "roundRect", 704, 236, 486, 268, WHITE, FAINT, 2, "rounded-xl");
  equation(slide, "s25_calibration", 728, 278, 438, 86, "Calibrated support subtracts the conditional null anchor and rescales by one minus that anchor.");
  text(slide, "Raw value + anchor + ratio\nmust travel together.", 732, 394, 430, 68, { size: 25, bold: true, color: TEAL, align: "center" });
  text(slide, "Different score vectors have different anchors → calibrated values cannot be subtracted.", 166, 540, 948, 66, { size: 23, bold: true, color: RED, align: "center" });
}

// 26
{
  const slide = newSlide("Subtracting two marginal reports combines\nadverse cases that never co-occurred", "Replacement", 26, ORANGE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 7.1 and Equation (19).");
  shape(slide, "roundRect", 92, 240, 450, 250, BLUE_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "MODEL A’S MARGINAL WORST CASE", 122, 270, 360, BLUE);
  text(slide, "site 1\n1% prevalence\nCNAP 0.30", 122, 324, 390, 126, { size: 29, bold: true, color: INK, align: "center" });
  shape(slide, "roundRect", 738, 240, 450, 250, ORANGE_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "MODEL B’S MARGINAL WORST CASE", 768, 270, 360, ORANGE);
  text(slide, "site 2\n30% prevalence\nCNAP 0.25", 768, 324, 390, 126, { size: 29, bold: true, color: INK, align: "center" });
  equation(slide, "s26_bad_subtraction", 430, 526, 420, 56, "The invalid marginal subtraction reports 0.05.");
  text(slide, "But no evaluation ever contained that comparison.", 352, 592, 576, 30, { size: 24, bold: true, color: RED, align: "center" });
}

// 27
{
  const slide = newSlide("Pairing can still reward\narbitrary tie-breaking", "Replacement", 27, ORANGE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 7.2.", 44);
  shape(slide, "roundRect", 74, 230, 500, 300, BLUE_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "MODEL A: FOUR DISTINCT SCORES", 104, 258, 350, BLUE);
  for (let i = 0; i < 4; i++) {
    shape(slide, "roundRect", 116 + i * 100, 326, 72, 80, WHITE, BLUE, 2, "rounded-xl");
    text(slide, (0.9 - i * 0.2).toFixed(1), 116 + i * 100, 348, 72, 30, { size: 23, bold: true, color: BLUE, align: "center" });
  }
  equation(slide, "s27_a", 132, 438, 384, 56, "Expected average precision for model A is 49 over 72.");
  shape(slide, "roundRect", 706, 230, 500, 300, PLUM_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "MODEL B: ONE COMPLETE TIE", 736, 258, 350, PLUM);
  shape(slide, "roundRect", 790, 326, 332, 80, WHITE, PLUM, 2, "rounded-xl");
  text(slide, "0.5     0.5     0.5     0.5", 808, 348, 296, 30, { size: 23, bold: true, color: PLUM, align: "center" });
  equation(slide, "s27_b", 790, 438, 332, 56, "Average precision for the complete tie is one half.");
  shape(slide, "roundRect", 250, 544, 780, 92, RED_LIGHT, "none", 0, "rounded-xl");
  text(slide, "No association—yet the expected paired difference is positive:", 276, 552, 728, 28, { size: 21, bold: true, color: RED, align: "center" });
  equation(slide, "s27_bias", 460, 578, 360, 50, "The expected chance-normalized difference is approximately 0.361.");
}

// 28
{
  const slide = newSlide("Tie averaging removes expected credit\nfor distinctions the model did not make", "Replacement", 28, TEAL,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 7.3 and Equation (20).");
  smallLabel(slide, "ONE TIED BLOCK WITH TWO POSITIVES", 88, 218, 420, MUTED);
  const seqs = [
    { x: 90, seq: ["+", "+", "−"], equation: "s28_ap1" },
    { x: 392, seq: ["+", "−", "+"], equation: "s28_ap2" },
    { x: 694, seq: ["−", "+", "+"], equation: "s28_ap3" },
  ];
  seqs.forEach((d) => {
    shape(slide, "roundRect", d.x, 270, 250, 188, WHITE, FAINT, 2, "rounded-xl");
    d.seq.forEach((s, i) => dot(slide, d.x + 40 + i * 58, 316, s === "+" ? BLUE : ORANGE, s));
    equation(slide, d.equation, d.x + 38, 388, 174, 48, "Average precision for this within-tie ordering.");
  });
  arrowText(slide, 974, 330, 42, MUTED);
  shape(slide, "roundRect", 1046, 270, 150, 188, TEAL_LIGHT, "none", 0, "rounded-xl");
  text(slide, "average", 1046, 300, 150, 28, { size: 19, bold: true, color: MUTED, align: "center" });
  text(slide, "0.81", 1046, 350, 150, 48, { size: 37, bold: true, color: TEAL, align: "center" });
  text(slide, "Useful refinement still helps when it aligns with outcomes; arbitrary refinement earns zero expected advantage.", 142, 548, 996, 64, { size: 24, bold: true, color: INK, align: "center" });
}

// 29
{
  const slide = newSlide("Form the paired advantage before\napplying either challenge", "Replacement", 29, TEAL,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 7.4 and Equation (22).", 44);
  const stages = [
    { x: 84, label: "PAIR", equation: "s29_pair", fill: BLUE_LIGHT, color: BLUE },
    { x: 384, label: "POPULATION", equation: "s29_population", fill: ORANGE_LIGHT, color: ORANGE },
    { x: 684, label: "REPLICATION", equation: "s29_replication", fill: PLUM_LIGHT, color: PLUM },
    { x: 984, label: "EXPECTATION", equation: "s29_expectation", fill: TEAL_LIGHT, color: TEAL },
  ];
  stages.forEach((d, i) => {
    shape(slide, "roundRect", d.x, 254, 220, 250, d.fill, "none", 0, "rounded-xl");
    smallLabel(slide, d.label, d.x + 18, 280, 184, d.color);
    equation(slide, d.equation, d.x + 12, 334, 196, 116, `${d.label.toLowerCase()} stage of the paired estimand.`);
    if (i < stages.length - 1) arrowText(slide, d.x + 224, 352, 28, MUTED);
  });
  callout(slide, "Same units → same prevalence → same replication", 342, 560, 596, 66, WHITE, INK, 25, true);
}

// 30
{
  const slide = newSlide("Tie neutrality needs an\nexchangeable-label reference law", "Replacement", 30, PLUM,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 7.5 and Definition 6.");
  text(slide, "Conditionally on both score vectors and the observed positive count,", 178, 224, 924, 38, { size: 27, color: MUTED, align: "center" });
  text(slide, "every label vector with that many positives is equally likely.", 172, 274, 936, 44, { size: 32, bold: true, color: PLUM, align: "center" });
  shape(slide, "roundRect", 94, 374, 510, 188, TEAL_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "DEFENSIBLE REFERENCE", 126, 402, 250, TEAL);
  text(slide, "i.i.d. held-out units\nunder a signal-free label law", 126, 456, 446, 70, { size: 27, bold: true, color: INK, align: "center" });
  shape(slide, "roundRect", 676, 374, 510, 188, RED_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "NOT AUTOMATIC", 708, 402, 220, RED);
  text(slide, "matched, clustered, or\nheterogeneous-risk designs", 708, 456, 446, 70, { size: 27, bold: true, color: INK, align: "center" });
  text(slide, "Without the law, the paired estimand remains descriptive—but the no-positive-anchor guarantee is lost.", 126, 600, 1028, 32, { size: 23, bold: true, color: RED, align: "center" });
}

// 31
{
  const slide = newSlide("Coherent replacement permits\na no-verdict region", "Replacement", 31, ORANGE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 7.6 and Equation (24).");
  equation(slide, "s31_constraint", 390, 212, 500, 72, "The paired advantages in opposite orientations sum to no more than zero.");
  text(slide, "Both orientations cannot be positive.", 380, 286, 520, 30, { size: 24, bold: true, color: TEAL, align: "center" });
  rule(slide, 154, 430, 972, MUTED, 5);
  const zeroX = 640;
  shape(slide, "ellipse", zeroX - 10, 418, 20, 20, INK);
  text(slide, "0", zeroX - 22, 452, 44, 26, { size: 20, bold: true, color: INK, align: "center" });
  const leftX = 480;
  const rightX = 820;
  shape(slide, "roundRect", leftX, 405, rightX - leftX, 50, ORANGE_LIGHT, "none", 0, "rounded-xl");
  shape(slide, "ellipse", leftX - 9, 418, 20, 20, ORANGE);
  shape(slide, "ellipse", rightX - 9, 418, 20, 20, ORANGE);
  equation(slide, "s31_left", leftX - 82, 480, 174, 44, "The advantage of A over B is minus 0.02.");
  equation(slide, "s31_right", rightX - 82, 480, 174, 44, "The negative of B over A is 0.03.");
  text(slide, "no verdict", 560, 414, 180, 30, { size: 22, bold: true, color: ORANGE, align: "center" });
  callout(slide, "The declared challenges may defeat superiority in both directions.", 276, 540, 728, 88, ORANGE_LIGHT, INK, 22, true);
}

// 32
{
  const slide = newSlide("Replacement needs a lower bound\nabove the decision margin", "Replacement", 32, TEAL,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 7.6 and Equation (25). Values are illustrative.");
  smallLabel(slide, "ILLUSTRATIVE RESULT", 92, 220, 260, MUTED);
  rule(slide, 126, 410, 1010, MUTED, 5);
  const x0 = 126;
  const scale = 8000;
  const marginX = x0 + 0.05 * scale;
  const estimateX = x0 + 0.08 * scale;
  const lowerX = x0 + 0.03 * scale;
  shape(slide, "roundRect", lowerX, 394, estimateX - lowerX, 38, BLUE_LIGHT, "none", 0, "rounded-xl");
  shape(slide, "ellipse", estimateX - 10, 398, 22, 22, BLUE);
  shape(slide, "rect", marginX - 3, 362, 6, 100, ORANGE);
  text(slide, "lower bound .03", lowerX - 82, 460, 164, 26, { size: 19, bold: true, color: BLUE, align: "center" });
  text(slide, "estimate .08", estimateX - 72, 334, 144, 26, { size: 19, bold: true, color: BLUE, align: "center" });
  text(slide, "decision margin .05", marginX - 96, 486, 192, 26, { size: 19, bold: true, color: ORANGE, align: "center" });
  callout(slide, "Do not replace: the outer lower bound does not exceed the decision margin.", 246, 550, 788, 68, RED_LIGHT, RED, 23, true);
  text(slide, "Performance evidence is one decision input; costs, fairness, calibration, and intervention effects remain separate.", 132, 244, 1016, 64, { size: 25, bold: true, color: INK, align: "center" });
}

// 33
{
  const slide = newSlide("The number is meaningful only together\nwith the assessment it survived", "Interpretation", 33, PLUM,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 8.1–8.2.");
  shape(slide, "roundRect", 96, 224, 1088, 92, TEAL_LIGHT, "none", 0, "rounded-xl");
  text(slide, "Supported quantity: raw support or paired advantage", 130, 250, 1020, 40, { size: 30, bold: true, color: TEAL, align: "center" });
  const items = [
    { y: 360, color: BLUE, label: "Π", body: "Which populations had to be survived?" },
    { y: 426, color: PLUM, label: "ℛ", body: "What changed in a complete replication?" },
    { y: 492, color: ORANGE, label: "K", body: "How many opportunities for defeat were joined?" },
    { y: 558, color: RED, label: "?", body: "Which invariance, symmetry, and uncertainty assumptions qualify the result?" },
  ];
  items.forEach((d) => {
    shape(slide, "ellipse", 132, d.y, 44, 44, d.color);
    text(slide, d.label, 132, d.y + 8, 44, 24, { size: 20, bold: true, color: WHITE, align: "center" });
    text(slide, d.body, 206, d.y + 5, 900, 34, { size: 25, bold: true, color: INK });
  });
}

// 34
{
  const slide = newSlide("Performance becomes a scientific claim\nonly when its challenges are declared", "Synthesis", 34, TEAL,
    "Stats_Paper_Extending_PRAUC_v12.tex, Conclusion.");
  const stages = [
    { x: 80, title: "Name populations", sub: "Standardize the prior\nand normalize", fill: BLUE_LIGHT, color: BLUE },
    { x: 370, title: "Expose failures", sub: "Minimum over target\nprevalence in each draw", fill: ORANGE_LIGHT, color: ORANGE },
    { x: 660, title: "Demand survival", sub: "Expected minimum\nover attempts", fill: PLUM_LIGHT, color: PLUM },
    { x: 950, title: "Make the claim", sub: "Paired, tie-neutral,\nmargin-qualified", fill: TEAL_LIGHT, color: TEAL },
  ];
  stages.forEach((d, i) => {
    shape(slide, "roundRect", d.x, 254, 240, 242, d.fill, "none", 0, "rounded-xl");
    text(slide, d.title, d.x + 20, 286, 200, 54, { size: 25, bold: true, color: d.color, align: "center" });
    text(slide, d.sub, d.x + 20, 380, 200, 72, { size: 23, bold: true, color: INK, align: "center" });
    if (i < stages.length - 1) arrowText(slide, d.x + 240, 356, 28, MUTED);
  });
  text(slide, "Support is not a new name for high performance.", 282, 548, 716, 36, { size: 29, bold: true, color: INK, align: "center" });
  text(slide, "It is the surviving magnitude after the challenges have been declared.", 178, 594, 924, 32, { size: 25, color: TEAL, align: "center" });
}

// 35 backup
{
  const slide = newSlide("Empirical prior-standardized AP is\na weighted threshold sum", "Backup", 35, BLUE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Appendix A, Equation (27).");
  shape(slide, "roundRect", 104, 224, 1072, 128, WHITE, FAINT, 2, "rounded-xl");
  equation(slide, "s35_ap", 128, 242, 1024, 92, "Empirical prior-standardized average precision is a recall-weighted threshold sum.");
  const items = [
    { y: 400, equation: "s35_recall", body: "recall gained when a score block enters" },
    { y: 478, equation: "s35_positive", body: "target-population positive mass selected" },
    { y: 556, equation: "s35_negative", body: "target-population negative mass selected" },
  ];
  items.forEach((d) => {
    equation(slide, d.equation, 136, d.y - 8, 250, 48, "Term from the empirical prior-standardized average-precision formula.");
    text(slide, d.body, 430, d.y + 2, 670, 32, { size: 24, color: INK });
  });
}

// 36 backup
{
  const slide = newSlide("More attempts strengthen the challenge\nbut do not broaden the regime", "Backup", 36, PLUM,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 4.5 and Appendix B.");
  addLineChart(slide, 86, 218, 676, 378, ["1", "2", "3", "4", "5"], [
    { name: "supported level", values: [52, 43, 38, 34, 31], line: { style: "solid", fill: PLUM, width: 4 }, marker: { symbol: "circle", size: 7 } },
  ], { legend: false, yMin: 0, yMax: 60, major: 10 });
  text(slide, "attempts →", 348, 596, 140, 24, { size: 18, bold: true, color: MUTED, align: "center" });
  shape(slide, "roundRect", 828, 236, 340, 300, PLUM_LIGHT, "none", 0, "rounded-xl");
  text(slide, "The attempt count controls", 842, 272, 312, 34, { size: 25, bold: true, color: PLUM, align: "center" });
  text(slide, "how many draws\nmust survive jointly", 858, 340, 280, 86, { size: 26, bold: true, color: INK, align: "center" });
  text(slide, "It does not decide whether\nsites or model fits vary.", 858, 458, 280, 54, { size: 19, color: MUTED, align: "center" });
}

// 37 backup
{
  const slide = newSlide("The two-attempt estimator avoids\nenumerating replication pairs", "Backup", 37, BLUE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Appendix F, Equation (46).");
  text(slide, "Sort the retained bootstrap effects", 166, 226, 948, 38, { size: 29, bold: true, color: INK, align: "center" });
  const zs = [".12", ".20", ".31", ".44", ".58"];
  zs.forEach((z, i) => {
    shape(slide, "roundRect", 230 + i * 170, 304, 130, 82, i < 2 ? ORANGE_LIGHT : BLUE_LIGHT, "none", 0, "rounded-xl");
    text(slide, `ordered ${i + 1}`, 230 + i * 170, 316, 130, 22, { size: 17, bold: true, color: MUTED, align: "center" });
    text(slide, z, 230 + i * 170, 346, 130, 28, { size: 25, bold: true, color: i < 2 ? ORANGE : BLUE, align: "center" });
  });
  shape(slide, "roundRect", 146, 460, 988, 100, WHITE, FAINT, 2, "rounded-xl");
  equation(slide, "s37_estimator", 176, 472, 928, 76, "Sorted-sample formula for the two-attempt support estimator.");
  text(slide, "Small values receive larger weights because they are the minimum in more pairs.", 232, 594, 816, 30, { size: 23, color: MUTED, align: "center" });
}

// 38 backup
{
  const slide = newSlide("Tie averaging can be computed\nwithout enumerating orderings", "Backup", 38, TEAL,
    "Stats_Paper_Extending_PRAUC_v12.tex, Appendix E.");
  shape(slide, "roundRect", 92, 230, 420, 286, TEAL_LIGHT, "none", 0, "rounded-xl");
  smallLabel(slide, "FOR EACH TIED BLOCK", 124, 260, 250, TEAL);
  text(slide, "1. Choose the position k\nof a focal positive\n\n2. Count t preceding positives\nwith a hypergeometric law", 124, 320, 356, 154, { size: 25, bold: true, color: INK, align: "left" });
  arrowText(slide, 572, 346, 48, MUTED);
  shape(slide, "roundRect", 686, 230, 502, 300, WHITE, FAINT, 2, "rounded-xl");
  text(slide, "Average its prior-standardized\nprecision over k and t", 726, 270, 422, 96, { size: 27, bold: true, color: INK, align: "center" });
  text(slide, "Computational cost", 726, 376, 422, 30, { size: 22, bold: true, color: MUTED, align: "center" });
  equation(slide, "s38_cost", 786, 402, 302, 50, "Tie-averaging computational cost is quadratic within tied blocks.");
  text(slide, "No enumeration or random jitter is required.", 726, 468, 422, 42, { size: 19, color: MUTED, align: "center" });
  text(slide, "The combinatorial weights do not depend on prevalence and can be reused during the search.", 172, 576, 936, 36, { size: 23, bold: true, color: INK, align: "center" });
}

// 39 backup
{
  const slide = newSlide("Outer uncertainty asks how well\none evaluation identifies the target", "Backup", 39, ORANGE,
    "Stats_Paper_Extending_PRAUC_v12.tex, Section 5.2 and Appendix F.4.");
  const stages = [
    { x: 98, title: "Observed evaluation", body: "one empirical score–label distribution", fill: BLUE_LIGHT, color: BLUE },
    { x: 430, title: "Outer sample", body: "new empirical distribution of units", fill: ORANGE_LIGHT, color: ORANGE },
    { x: 762, title: "Full inner procedure", body: "profile → infimum → support", fill: PLUM_LIGHT, color: PLUM },
  ];
  stages.forEach((d, i) => {
    shape(slide, "roundRect", d.x, 266, 280, 218, d.fill, "none", 0, "rounded-xl");
    text(slide, d.title, d.x + 20, 300, 240, 58, { size: 25, bold: true, color: d.color, align: "center" });
    text(slide, d.body, d.x + 24, 390, 232, 62, { size: 22, color: INK, align: "center" });
    if (i < stages.length - 1) arrowText(slide, d.x + 282, 350, 32, MUTED);
  });
  arrowText(slide, 1074, 350, 32, MUTED);
  shape(slide, "roundRect", 1120, 286, 90, 178, TEAL_LIGHT, "none", 0, "rounded-xl");
  text(slide, "lower\nbound", 1120, 334, 90, 70, { size: 23, bold: true, color: TEAL, align: "center" });
  text(slide, "Coverage is not automatic when the minimizing prevalence is flat or moves.", 180, 560, 920, 52, { size: 25, bold: true, color: RED, align: "center" });
}

// 40 references
{
  const slide = newSlide("Key references", "Backup", 40, BLUE,
    "Stats_Paper_Extending_PRAUC_v12.tex, bibliography.");
  const left = [
    "Popper, K. (1959). The Logic of Scientific Discovery.",
    "Popper, K. (1963). Conjectures and Refutations.",
    "Siblini et al. (2020). Master your metrics with calibration.",
    "Elkan, C. (2001). Foundations of cost-sensitive learning.",
    "Saerens et al. (2002). Adjusting outputs to new priors.",
  ];
  const right = [
    "Bestgen, Y. (2015). Exact expected AP of the random baseline.",
    "Ojala & Garriga (2010). Permutation tests for classifier performance.",
    "Bareinboim & Pearl (2013). Transportability of experimental results.",
    "Hoeffding, W. (1948). U-statistics.",
    "Perdomo et al. (2020). Performative prediction.",
  ];
  smallLabel(slide, "POPULATION + CORROBORATION", 84, 220, 400, BLUE);
  left.forEach((r, i) => text(slide, r, 84, 266 + i * 68, 520, 48, { size: 20, color: INK }));
  smallLabel(slide, "DESIGN + COMPUTATION", 684, 220, 400, TEAL);
  right.forEach((r, i) => text(slide, r, 684, 266 + i * 68, 520, 48, { size: 20, color: INK }));
  text(slide, "Complete references appear in Stats_Paper_Extending_PRAUC_v12.tex.", 246, 626, 788, 28, { size: 20, color: MUTED, align: "center" });
}

// Add meaningful speaker notes to slides whose notes were initially blank.
const talks = [
  null,
  "Contrast a ranking statistic with the object that downstream decisions actually consume: a selected set.",
  "Define the two axes before showing a PR curve. Emphasize that neither is a threshold-free property.",
  "Walk the threshold down the list. The curve is generated by a sequence of selected sets.",
  "Calculate AP directly from the three positive ranks. This makes the later weighted formula less mysterious.",
  "Use the numerical contrast to create the central population tension. TPR and FPR are unchanged.",
  "State the transformation criterion before presenting the formula: preserve class-conditional behavior; alter only the prior.",
  "Substitute r=.80 and f=.05. The values reproduce the slide 6 precision contrast and fill in intermediate populations.",
  "Draw a firm boundary around the reweighting claim. If score behavior within class changes, prevalence must be represented through environments in the regime.",
  "Explain why raw AP values at different prevalences do not share a random baseline.",
  "Normalization makes zero and one common, but it does not erase how false positives matter differently as prevalence changes.",
  "Ask whether the scientific claim is an expectation across populations or a level that must hold in all of them. The infimum answers only the latter.",
  "Introduce the assessment only now that both population and repetition questions are visible.",
  "A bootstrap is not a scientific target. The regime names the target; resampling must then represent it.",
  "Resolve the apparent asymmetry. Prevalence is part of the same assessment, but invariance lets it be computed within a realized replication.",
  "Pause on the failed average. The number 0.375 summarizes compensation, not survival.",
  "The realized minimum is the greatest level all K challenges retain. Expectation keeps the result on the performance scale.",
  "At K=2 the operator has a transparent decomposition. This is not a standard-error penalty.",
  "Use Popper carefully: the language is negative and conditional. Support records survival under stated tests; it is not confirmation.",
  "Separate B, K, and genuinely new evidence. This prevents computational effort from being mistaken for severity.",
  "The different profile shapes show why each replication must face the entire prevalence set before replications are combined.",
  "The numerical decomposition is illustrative. Its job is diagnostic: distinguish weak mean performance, moving limiting populations, and unstable retained levels.",
  "Walk left to right. The same sampled units must generate the complete prevalence profile.",
  "This two-unit example proves that a population chance anchor does not automatically center a finite design.",
  "Calibration is useful for describing one fixed model. Stress the non-composability of different conditional anchors.",
  "A replacement effect must be realized under the same conditions. Marginal worst cases do not supply it.",
  "Both models are uninformative. The finer score vector nevertheless receives positive expected advantage under ordinary AP.",
  "Tie averaging changes the convention rather than subtracting a null artifact. Arbitrary within-block order is averaged away.",
  "Build the estimand from left to right. Pair first; challenge second.",
  "The reference guarantee has a real design assumption. Pairing and tie averaging do not create exchangeability.",
  "The operator is not odd, but the orientation inequality prevents contradictory positive recommendations. It still allows honest indecision.",
  "A positive estimate is insufficient. The uncertainty-qualified effect must clear a prespecified practical margin.",
  "Model performance is assessment-relative. Reporting only the scalar removes the conditions that gave it meaning.",
  "Return to the opening question. The framework’s contribution is a claim structure that can be criticized and improved.",
  "Use this only if the audience wants the exact empirical AP definition.",
  "K and the regime control different aspects of severity. The values in the chart are illustrative.",
  "The sorted formula gives the complete order-two U-statistic efficiently.",
  "The closed form integrates over positions and within-block label composition; the visible slide intentionally omits the full combinatorial expression.",
  "Outer resampling targets inferential uncertainty about the supported functional, not replication variation itself.",
  "Use this slide for questions about intellectual lineage and technical sources.",
];

presentation.slides.items.forEach((slide, i) => {
  if (i === 0) return;
  const current = slide.speakerNotes.textFrame.text ?? "";
  const sourceBlock = current.includes("[Sources]") ? current.slice(current.indexOf("[Sources]")) : "[Sources]\n- Stats_Paper_Extending_PRAUC_v12.tex.";
  slide.speakerNotes.textFrame.setText(`${talks[i] ?? ""}\n\n${sourceBlock}`);
});

await fs.mkdir(`${BUILD}/rendered`, { recursive: true });
await fs.mkdir(`${BUILD}/layouts`, { recursive: true });

for (const [index, slide] of presentation.slides.items.entries()) {
  const stem = `slide-${String(index + 1).padStart(2, "0")}`;
  const png = await presentation.export({ slide, format: "png", scale: 1.5 });
  await fs.writeFile(`${BUILD}/rendered/${stem}.png`, new Uint8Array(await png.arrayBuffer()));
  const layout = await slide.export({ format: "layout" });
  await fs.writeFile(`${BUILD}/layouts/${stem}.layout.json`, await layout.text());
}

const montage = await presentation.export({ format: "webp", montage: true, scale: 0.55 });
await fs.writeFile(`${BUILD}/montage.webp`, new Uint8Array(await montage.arrayBuffer()));

const pptx = await PresentationFile.exportPptx(presentation);
await pptx.save(OUT);

console.log(`Wrote ${OUT}`);
console.log(`Slides: ${presentation.slides.items.length}`);
