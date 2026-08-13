import fs from "node:fs/promises";
import { Presentation, PresentationFile, layers, text } from "@oai/artifact-tool";

const OUT_DIR = "/Users/thomm15/Documents/Stats_Paper_AUPRC_extension/.tmp/prauc_narrative_revision/rendered";
const FINAL_PPTX = "/Users/thomm15/Documents/Stats_Paper_AUPRC_extension/.tmp/prauc_narrative_revision/candidate.pptx";
const SOURCE = "Stats_Paper_Extending_PRAUC_v12.tex";

const C = {
  ink: "#102436",
  muted: "#5B6B78",
  blue: "#2F80ED",
  blueSoft: "#EAF3FF",
  teal: "#0F9D91",
  tealSoft: "#E6F7F4",
  coral: "#E76F51",
  coralSoft: "#FCEDE8",
  gold: "#D79B22",
  goldSoft: "#FFF6DD",
  gray: "#F3F5F7",
  line: "#CAD3DB",
  white: "#FFFFFF",
  dark: "#0B1F33",
};

const FONT = "Helvetica Neue";
const W = 1280;
const H = 720;

async function writeBlob(path, blob) {
  await fs.writeFile(path, new Uint8Array(await blob.arrayBuffer()));
}

function addShape(slide, name, geometry, position, fill = C.white, lineFill = "none", lineWidth = 0) {
  return slide.shapes.add({
    geometry,
    name,
    position,
    fill,
    line: { style: "solid", fill: lineFill, width: lineWidth },
  });
}

function addText(slide, name, value, position, opts = {}) {
  const shape = slide.shapes.add({
    geometry: "textbox",
    name,
    position,
    fill: "none",
    line: { style: "solid", fill: "none", width: 0 },
  });
  shape.text = value;
  shape.text.style = {
    fontSize: opts.fontSize ?? 22,
    typeface: opts.typeface ?? FONT,
    color: opts.color ?? C.ink,
    bold: opts.bold ?? false,
    italic: opts.italic ?? false,
    alignment: opts.alignment ?? "left",
    verticalAlignment: opts.verticalAlignment ?? "top",
    autoFit: opts.autoFit ?? "none",
    insets: opts.insets ?? { top: 0, right: 0, bottom: 0, left: 0 },
  };
  return shape;
}

function addMath(slide, name, latex, position, opts = {}) {
  slide.compose(
    layers({ name: `${name}-layer`, width: "fill", height: "fill" }, [
      text([{ latex, displayMode: "block" }], {
        name,
        position: { left: position.left, top: position.top },
        width: position.width,
        height: position.height,
        style: {
          fontSize: `${opts.fontSize ?? 34}px`,
          typeface: opts.typeface ?? "Cambria Math",
          color: opts.color ?? C.dark,
          alignment: opts.alignment ?? "center",
          verticalAlignment: opts.verticalAlignment ?? "middle",
          autoFit: opts.autoFit ?? "shrinkText",
          insets: { top: 0, right: 0, bottom: 0, left: 0 },
        },
      }),
    ]),
    { frame: { left: 0, top: 0, width: W, height: H }, baseUnit: 1 },
  );
}

function notes(slide, locator) {
  slide.speakerNotes.textFrame.setText(
    `[Sources]\n- User-provided manuscript: ${SOURCE} — ${locator}`
  );
  slide.speakerNotes.setVisible(true);
}

function addChrome(slide, title, section, page, accent = C.blue) {
  slide.background.fill = C.white;
  addText(slide, `title-${page}`, title, { left: 42, top: 34, width: 1196, height: 82 }, {
    fontSize: 38, bold: true, color: C.dark, autoFit: "shrinkText",
  });
  addShape(slide, `rule-${page}`, "rect", { left: 42, top: 132, width: 1196, height: 2 }, accent);
  addText(slide, `deck-footer-${page}`, "Supported model comparison across target prevalences", {
    left: 42, top: 670, width: 640, height: 18,
  }, { fontSize: 12, color: C.muted });
  addText(slide, `page-${page}`, String(page), { left: 1184, top: 665, width: 54, height: 22 }, {
    fontSize: 13, color: C.muted, alignment: "right",
  });
}

function addTitleSlide(presentation, page) {
  const slide = presentation.slides.add();
  slide.background.fill = C.white;
  addText(slide, "deck-title", "Supported model comparison\nacross target prevalences", {
    left: 42, top: 145, width: 1030, height: 270,
  }, { fontSize: 68, bold: true, color: C.dark, verticalAlignment: "bottom" });
  addText(slide, "deck-subtitle", "A replication-based method for average precision", {
    left: 42, top: 485, width: 800, height: 60,
  }, { fontSize: 30, color: C.muted });
  addShape(slide, "title-accent", "rect", { left: 42, top: 565, width: 240, height: 7 }, C.blue);
  addText(slide, "author", "Marcus A. Thomas  ·  manuscript v12", {
    left: 42, top: 600, width: 620, height: 34,
  }, { fontSize: 18, color: C.muted });
  addText(slide, "page-title", String(page), { left: 1184, top: 665, width: 54, height: 22 }, {
    fontSize: 13, color: C.muted, alignment: "right",
  });
  notes(slide, "Title and abstract");
  return slide;
}

function addTwoColumnSlide(presentation, cfg, page) {
  const slide = presentation.slides.add();
  addChrome(slide, cfg.title, cfg.section, page, cfg.accent);
  const left = { left: 42, top: 185, width: 565, height: 420 };
  const right = { left: 650, top: 185, width: 588, height: 420 };
  addShape(slide, `left-panel-${page}`, "roundRect", left, cfg.leftFill ?? C.gray, C.line, 1);
  addShape(slide, `right-panel-${page}`, "roundRect", right, cfg.rightFill ?? C.gray, C.line, 1);
  addText(slide, `left-kicker-${page}`, cfg.leftKicker ?? "", { left: 74, top: 215, width: 500, height: 30 }, {
    fontSize: 15, bold: true, color: cfg.accent ?? C.blue,
  });
  if (cfg.leftLatex) {
    addMath(slide, `left-head-${page}`, cfg.leftLatex, { left: 74, top: 245, width: 500, height: 100 }, {
      fontSize: cfg.leftHeadSize ?? 32,
    });
  } else {
    addText(slide, `left-head-${page}`, cfg.leftHead, { left: 74, top: 255, width: 500, height: 82 }, {
      fontSize: cfg.leftHeadSize ?? 32, bold: true, color: C.dark, autoFit: "shrinkText",
    });
  }
  addText(slide, `left-body-${page}`, cfg.leftBody, { left: 74, top: 355, width: 500, height: 205 }, {
    fontSize: cfg.bodySize ?? 21, color: C.ink, autoFit: "shrinkText",
  });
  addText(slide, `right-kicker-${page}`, cfg.rightKicker ?? "", { left: 682, top: 215, width: 500, height: 30 }, {
    fontSize: 15, bold: true, color: cfg.accent ?? C.blue,
  });
  if (cfg.rightLatex) {
    addMath(slide, `right-head-${page}`, cfg.rightLatex, { left: 682, top: 245, width: 520, height: 100 }, {
      fontSize: cfg.rightHeadSize ?? 32,
    });
  } else {
    addText(slide, `right-head-${page}`, cfg.rightHead, { left: 682, top: 255, width: 520, height: 82 }, {
      fontSize: cfg.rightHeadSize ?? 32, bold: true, color: C.dark, autoFit: "shrinkText",
    });
  }
  addText(slide, `right-body-${page}`, cfg.rightBody, { left: 682, top: 355, width: 520, height: 205 }, {
    fontSize: cfg.bodySize ?? 21, color: C.ink, autoFit: "shrinkText",
  });
  notes(slide, cfg.source);
  return slide;
}

function addFormulaSlide(presentation, cfg, page) {
  const slide = presentation.slides.add();
  addChrome(slide, cfg.title, cfg.section, page, cfg.accent);
  addShape(slide, `formula-panel-${page}`, "roundRect", { left: 42, top: 190, width: 660, height: 380 },
    cfg.formulaFill ?? C.blueSoft, cfg.accent ?? C.blue, 1.5);
  addText(slide, `formula-label-${page}`, cfg.formulaLabel ?? "FORM", { left: 76, top: 220, width: 220, height: 28 }, {
    fontSize: 15, bold: true, color: cfg.accent ?? C.blue,
  });
  if (cfg.latexLines) {
    const lineHeight = 178 / cfg.latexLines.length;
    cfg.latexLines.forEach((line, index) => {
      addMath(slide, `formula-${page}-${index}`, line, {
        left: 76,
        top: 278 + index * lineHeight,
        width: 592,
        height: lineHeight,
      }, {
        fontSize: cfg.formulaSizes?.[index] ?? cfg.formulaSize ?? 30,
        alignment: cfg.formulaAlign ?? "center",
      });
    });
  } else {
    addMath(slide, `formula-${page}`, cfg.latex, { left: 76, top: 270, width: 592, height: 200 }, {
      fontSize: cfg.formulaSize ?? 34, alignment: cfg.formulaAlign ?? "center",
    });
  }
  if (cfg.formulaCaption) {
    addText(slide, `formula-caption-${page}`, cfg.formulaCaption, { left: 76, top: 488, width: 592, height: 48 }, {
      fontSize: 18, color: C.muted, alignment: "center", autoFit: "shrinkText",
    });
  }
  addText(slide, `meaning-label-${page}`, cfg.meaningLabel ?? "READ IT AS", { left: 760, top: 215, width: 420, height: 28 }, {
    fontSize: 15, bold: true, color: cfg.accent ?? C.blue,
  });
  addText(slide, `meaning-${page}`, cfg.meaning, { left: 760, top: 265, width: 430, height: 130 }, {
    fontSize: 27, bold: true, color: C.dark, autoFit: "shrinkText",
  });
  addText(slide, `why-${page}`, cfg.why, { left: 760, top: 430, width: 430, height: 140 }, {
    fontSize: 21, color: C.ink, autoFit: "shrinkText",
  });
  if (cfg.bottomLatex) {
    addMath(slide, `bottom-${page}`, cfg.bottomLatex, { left: 42, top: 596, width: 1196, height: 55 }, {
      fontSize: cfg.bottomSize ?? 24,
    });
  } else if (cfg.bottom) {
    addText(slide, `bottom-${page}`, cfg.bottom, { left: 42, top: 605, width: 1196, height: 40 }, {
      fontSize: 18, color: C.muted, alignment: "center", autoFit: "shrinkText",
    });
  }
  notes(slide, cfg.source);
  return slide;
}

function addThreeColumnSlide(presentation, cfg, page) {
  const slide = presentation.slides.add();
  addChrome(slide, cfg.title, cfg.section, page, cfg.accent);
  const xs = [42, 452, 862];
  cfg.items.forEach((item, i) => {
    addShape(slide, `card-${page}-${i}`, "roundRect", { left: xs[i], top: 205, width: 376, height: 390 },
      item.fill ?? C.gray, C.line, 1);
    addText(slide, `num-${page}-${i}`, item.kicker ?? `0${i + 1}`, {
      left: xs[i] + 30,
      top: 235,
      width: item.kickerWidth ?? 70,
      height: 28,
    }, {
      fontSize: 15, bold: true, color: cfg.accent ?? C.blue,
    });
    addText(slide, `head-${page}-${i}`, item.head, { left: xs[i] + 30, top: 285, width: 316, height: 88 }, {
      fontSize: 27, bold: true, color: C.dark, autoFit: "shrinkText",
    });
    addText(slide, `body-${page}-${i}`, item.body, { left: xs[i] + 30, top: 395, width: 316, height: 155 }, {
      fontSize: 20, color: C.ink, autoFit: "shrinkText",
    });
  });
  if (cfg.bottomLatex) {
    addMath(slide, `bottom-${page}`, cfg.bottomLatex, { left: 42, top: 600, width: 1196, height: 48 }, {
      fontSize: cfg.bottomSize ?? 24,
    });
  } else if (cfg.bottom) {
    addText(slide, `bottom-${page}`, cfg.bottom, { left: 42, top: 610, width: 1196, height: 38 }, {
      fontSize: 18, color: C.muted, alignment: "center", autoFit: "shrinkText",
    });
  }
  notes(slide, cfg.source);
  return slide;
}

function addProcessSlide(presentation, cfg, page) {
  const slide = presentation.slides.add();
  addChrome(slide, cfg.title, cfg.section, page, cfg.accent);
  const n = cfg.steps.length;
  const start = 62;
  const gap = 28;
  const cardW = (1156 - gap * (n - 1)) / n;
  const top = 245;
  for (let i = 0; i < n - 1; i++) {
    addShape(slide, `arrow-${page}-${i}`, "rightArrow", {
      left: start + cardW - 2 + i * (cardW + gap), top: 380, width: gap + 4, height: 22,
    }, cfg.accent ?? C.blue);
  }
  cfg.steps.forEach((item, i) => {
    const x = start + i * (cardW + gap);
    addShape(slide, `step-${page}-${i}`, "roundRect", { left: x, top, width: cardW, height: 310 },
      item.fill ?? C.gray, C.line, 1);
    addText(slide, `step-num-${page}-${i}`, String(i + 1), { left: x + 24, top: top + 24, width: 36, height: 30 }, {
      fontSize: 16, bold: true, color: cfg.accent ?? C.blue,
    });
    addText(slide, `step-head-${page}-${i}`, item.head, { left: x + 24, top: top + 78, width: cardW - 48, height: 76 }, {
      fontSize: 25, bold: true, color: C.dark, autoFit: "shrinkText",
    });
    addText(slide, `step-body-${page}-${i}`, item.body, { left: x + 24, top: top + 175, width: cardW - 48, height: 95 }, {
      fontSize: 18, color: C.ink, autoFit: "shrinkText",
    });
  });
  if (cfg.lead) {
    addText(slide, `lead-${page}`, cfg.lead, { left: 62, top: 172, width: 1120, height: 48 }, {
      fontSize: 22, color: C.muted, alignment: "center", autoFit: "shrinkText",
    });
  }
  notes(slide, cfg.source);
  return slide;
}

function addProofSlide(presentation, cfg, page) {
  const slide = presentation.slides.add();
  addChrome(slide, cfg.title, cfg.section, page, cfg.accent);
  addShape(slide, `identity-${page}`, "roundRect", { left: 42, top: 205, width: 510, height: 375 },
    cfg.formulaFill ?? C.tealSoft, cfg.accent ?? C.teal, 1.5);
  addText(slide, `identity-label-${page}`, cfg.formulaLabel ?? "KEY IDENTITY", { left: 74, top: 235, width: 250, height: 28 }, {
    fontSize: 15, bold: true, color: cfg.accent ?? C.teal,
  });
  addMath(slide, `identity-formula-${page}`, cfg.latex, { left: 74, top: 285, width: 446, height: 220 }, {
    fontSize: cfg.formulaSize ?? 31,
  });
  const ys = [215, 355, 495];
  cfg.steps.forEach((s, i) => {
    addText(slide, `proof-num-${page}-${i}`, `${i + 1}`, { left: 610, top: ys[i], width: 35, height: 32 }, {
      fontSize: 16, bold: true, color: cfg.accent ?? C.teal,
    });
    addText(slide, `proof-head-${page}-${i}`, s.head, { left: 660, top: ys[i], width: 520, height: 36 }, {
      fontSize: 24, bold: true, color: C.dark,
    });
    addText(slide, `proof-body-${page}-${i}`, s.body, { left: 660, top: ys[i] + 48, width: 520, height: 62 }, {
      fontSize: 18, color: C.ink, autoFit: "shrinkText",
    });
  });
  notes(slide, cfg.source);
  return slide;
}

function addLineChartSlide(presentation, cfg, page) {
  const slide = presentation.slides.add();
  addChrome(slide, cfg.title, cfg.section, page, cfg.accent);
  addShape(slide, `chart-frame-${page}`, "roundRect", { left: 42, top: 180, width: 690, height: 445 },
    C.white, C.line, 1);
  slide.charts.add("line", {
    position: { left: 70, top: 215, width: 635, height: 365 },
    categories: cfg.categories,
    series: cfg.series,
    hasLegend: true,
    legend: { position: "bottom", overlay: false },
    chartFill: C.white,
    chartLine: { style: "solid", width: 0, fill: C.white },
    plotAreaFill: { type: "none" },
    plotAreaLine: { style: "solid", width: 0, fill: C.white },
    xAxis: {
      visible: true,
      deleted: false,
      line: { style: "solid", width: 1, fill: C.line },
      textStyle: { typeface: FONT, fontSize: "13px", color: C.ink },
    },
    yAxis: {
      visible: true,
      deleted: false,
      min: cfg.yMin ?? 0,
      max: cfg.yMax ?? 1,
      majorUnit: cfg.majorUnit ?? 0.2,
      majorGridlines: { style: "solid", width: 1, fill: "#E8EDF1" },
      line: { style: "solid", width: 0, fill: C.white },
      textStyle: { typeface: FONT, fontSize: "13px", color: C.ink },
    },
  });
  addText(slide, `chart-note-label-${page}`, cfg.sideLabel ?? "INTUITION", {
    left: 790, top: 215, width: 380, height: 28,
  }, { fontSize: 15, bold: true, color: cfg.accent ?? C.blue });
  if (cfg.sideLatex) {
    addMath(slide, `chart-note-${page}`, cfg.sideLatex, {
      left: 790, top: 250, width: 410, height: 125,
    }, { fontSize: 30 });
  } else {
    addText(slide, `chart-note-${page}`, cfg.sideHead, {
      left: 790, top: 265, width: 410, height: 105,
    }, { fontSize: 30, bold: true, color: C.dark, autoFit: "shrinkText" });
  }
  addText(slide, `chart-body-${page}`, cfg.sideBody, {
    left: 790, top: 405, width: 410, height: 150,
  }, { fontSize: 21, color: C.ink, autoFit: "shrinkText" });
  addText(slide, `schematic-${page}`, "Schematic profile — not empirical study data", {
    left: 790, top: 585, width: 410, height: 28,
  }, { fontSize: 14, italic: true, color: C.muted });
  notes(slide, cfg.source);
  return slide;
}

function addDividerSlide(presentation, cfg, page) {
  const slide = presentation.slides.add();
  slide.background.fill = C.dark;
  addText(slide, `divider-title-${page}`, cfg.title, { left: 42, top: 175, width: 1100, height: 290 }, {
    fontSize: 68, bold: true, color: C.white, verticalAlignment: "middle", autoFit: "shrinkText",
  });
  addText(slide, `divider-sub-${page}`, cfg.subtitle, { left: 42, top: 545, width: 900, height: 72 }, {
    fontSize: 24, color: "#C7D4E0", autoFit: "shrinkText",
  });
  addText(slide, `divider-page-${page}`, String(page), { left: 1184, top: 665, width: 54, height: 22 }, {
    fontSize: 13, color: "#C7D4E0", alignment: "right",
  });
  notes(slide, cfg.source);
  return slide;
}

const presentation = Presentation.create({ slideSize: { width: W, height: H } });
let p = 1;

addTitleSlide(presentation, p++);

addTwoColumnSlide(presentation, {
  title: "Precision changes with prevalence even when ranking does not",
  section: "The problem",
  accent: C.coral,
  leftFill: C.tealSoft,
  rightFill: C.coralSoft,
  leftKicker: "50% PREVALENCE",
  leftHead: "Precision ≈ 0.94",
  leftBody: "10,000 cases\n4,000 true positives\n250 false positives\n\nTPR = 80%  ·  FPR = 5%",
  rightKicker: "1% PREVALENCE",
  rightHead: "Precision ≈ 0.14",
  rightBody: "10,000 cases\n80 true positives\n495 false positives\n\nThe ranking rates did not change.",
  source: "Section “The problem: what did the performance claim survive?”",
}, p++);

addThreeColumnSlide(presentation, {
  title: "A portable claim must define its target set, replication law, and K",
  section: "The problem",
  accent: C.blue,
  items: [
    { kicker: "Π★", head: "Where must it hold?", body: "The target prevalence set: named populations or a defensible interval.", fill: C.blueSoft },
    { kicker: "ℛ", head: "What may change?", body: "The law for one complete replication: units, environments, measurement, or development.", fill: C.tealSoft },
    { kicker: "K", head: "How many chances to fail?", body: "Independent replications that must retain the claimed level jointly.", fill: C.goldSoft },
  ],
  bottomLatex: String.raw`\mathrm{A}=(Π_{∗},\mathrm{R},K)`,
  source: "Section “The problem: what did the performance claim survive?”",
}, p++);

addFormulaSlide(presentation, {
  title: "How can prevalence change without changing the ranking?",
  section: "Problem I · Prior standardization",
  accent: C.blue,
  latex: String.raw`p_π(c)=\frac{π r(c)}{π r(c)+(1-π)f(c)}`,
  formulaSize: 34,
  formulaCaption: "r(c): true-positive rate   ·   f(c): false-positive rate",
  meaning: "Reweight the class prior to the target prevalence π.",
  why: "The observed class-conditional ranking rates stay fixed. Applying this at every score threshold produces APπ.",
  source: "Subsection “Prior standardization”",
}, p++);

addFormulaSlide(presentation, {
  title: "Valid reweighting must separate prior odds from ranking evidence",
  section: "Problem I · Prior standardization",
  accent: C.blue,
  latex: String.raw`\frac{p_π(c)}{1-p_π(c)}=\frac{π}{1-π}\frac{r(c)}{f(c)}`,
  formulaSize: 32,
  meaning: "Population prior × ranking evidence.",
  why: "Only the first factor changes with the target population. The second comes from the score distributions within class.",
  source: "Subsection “Prior standardization”",
}, p++);

addTwoColumnSlide(presentation, {
  title: "Prior reweighting requires stable within-class score behavior",
  section: "Problem I · What licenses the transformation",
  accent: C.teal,
  leftFill: C.tealSoft,
  rightFill: C.coralSoft,
  leftKicker: "LICENSED",
  leftHead: "Pobs(S≥c | Y=y)\n= Ptarget(S≥c | Y=y)",
  leftLatex: String.raw`P_{\mathrm{obs}}(S≥c\,|\,Y=y)=P_{\mathrm{target}}(S≥c\,|\,Y=y)`,
  leftHeadSize: 28,
  leftBody: "Target populations differ in prevalence, while within-class score behavior is stable.",
  rightKicker: "NOT LICENSED",
  rightHead: "Severity, measurement, or within-class behavior changes",
  rightHeadSize: 27,
  rightBody: "Then prevalence must enter through environments generated by ℛ. Reweighting one evaluation cannot transport the ranking.",
  source: "Subsection “What licenses the transformation”",
}, p++);

addFormulaSlide(presentation, {
  title: "Chance-Normalized AP changes the prior—not the ranking evidence",
  section: "Problem I · A common effect scale",
  accent: C.teal,
  formulaFill: C.tealSoft,
  latex: String.raw`\operatorname{CNAP}_π(D)=\frac{\operatorname{AP}_π(D)-π}{1-π}`,
  formulaSize: 36,
  formulaCaption: "0 = population random ranking   ·   1 = perfect ranking",
  meaning: "Subtract the prevalence-specific chance level, then rescale to the ceiling.",
  why: "Common endpoints improve interpretation—but the profile can still change with prevalence.",
  source: "Subsection “A common effect scale”",
}, p++);

addLineChartSlide(presentation, {
  title: "Robust performance is the minimum normalized AP across prevalences",
  section: "Problem I · From a profile to a claim",
  accent: C.blue,
  categories: ["1%", "5%", "10%", "25%", "50%"],
  series: [
    { name: "Model A", categories: ["1%", "5%", "10%", "25%", "50%"], values: [0.18, 0.31, 0.40, 0.49, 0.54],
      line: { style: "solid", width: 3, fill: C.blue }, marker: { symbol: "circle", size: 6 } },
    { name: "Model B", categories: ["1%", "5%", "10%", "25%", "50%"], values: [0.25, 0.34, 0.38, 0.41, 0.43],
      line: { style: "solid", width: 3, fill: C.teal }, marker: { symbol: "circle", size: 6 } },
  ],
  yMin: 0, yMax: 0.6, majorUnit: 0.1,
  sideLabel: "ROBUST CNAP",
  sideHead: "robust CNAP",
  sideLatex: String.raw`\inf_{π∈Π_{∗}}\operatorname{CNAP}_π(D)`,
  sideBody: "The largest level attained at every declared prevalence. Broadening Π★ can only keep or lower this value.",
  source: "Subsection “From a profile to a claim”",
}, p++);

addThreeColumnSlide(presentation, {
  title: "What, exactly, is allowed to vary across replications?",
  section: "Problem II · The evaluation-generating regime",
  accent: C.coral,
  items: [
    { kicker: "FIXED MODEL", kickerWidth: 316, head: "New evaluation units", body: "Hold the environment, measurement process, and fitted model fixed.", fill: C.gray },
    { kicker: "NEW ENVIRONMENT", kickerWidth: 316, head: "New deployment setting", body: "Redraw the environment and declare how measurement changes.", fill: C.coralSoft },
    { kicker: "REPEATED DEVELOPMENT", kickerWidth: 316, head: "Redevelop the model", body: "Redraw training, tuning, selection, fitting, and evaluation.", fill: C.goldSoft },
  ],
  bottom: "The scientific claim changes when the source of variation changes.",
  source: "Subsection “The evaluation-generating regime”",
}, p++);

addTwoColumnSlide(presentation, {
  title: "Prevalence and replication require different challenge operations",
  section: "Problem II · One assessment, two modes",
  accent: C.teal,
  leftFill: C.blueSoft,
  rightFill: C.tealSoft,
  leftKicker: "WITHIN EACH REPLICATION",
  leftHead: "Interrogate every π ∈ Π★",
  leftBody: "When score-level invariance holds, one realized evaluation supplies the rates needed for all target prevalences.",
  rightKicker: "ACROSS REPLICATIONS",
  rightHead: "Generate draws from ℛ",
  rightBody: "Environments, measurement conditions, and development runs cannot be recovered by prevalence reweighting.",
  source: "Subsection “One assessment, two modes of challenge”",
}, p++);

addFormulaSlide(presentation, {
  title: "Should strong replications compensate for a failed one?",
  section: "Problem III · From a failed average to joint survival",
  accent: C.coral,
  formulaFill: C.coralSoft,
  latex: String.raw`S_K(T;\mathrm{R})=\operatorname{E}_{\mathrm{R}}\left[\min_{1≤j≤K}T_j\right]`,
  formulaSize: 38,
  formulaCaption: "Example: min(0.70, 0.05) = 0.05, while the mean is 0.375",
  meaning: "A stronger replication cannot rescue a level defeated by a weaker one.",
  why: "At K=2, support equals expected performance minus half the expected absolute disagreement between replications.",
  source: "Subsection “From a failed average to joint survival”",
}, p++);

addThreeColumnSlide(presentation, {
  title: "K changes the claim—not computation or new evidence",
  section: "Problem III · Projected support and empirical corroboration",
  accent: C.gold,
  items: [
    { kicker: "B", head: "Computational replications", body: "More bootstrap draws reduce Monte Carlo error for the same target.", fill: C.gray },
    { kicker: "K", head: "Required joint survival", body: "Changing K changes the claim and its severity.", fill: C.goldSoft },
    { kicker: "NEW DATA", kickerWidth: 150, head: "Empirical replication", body: "New environments or development runs can expose failures the bootstrap cannot see.", fill: C.tealSoft },
  ],
  bottom: "The estimate projects what would survive K executions of the declared regime.",
  source: "Subsection “Projected support and empirical corroboration”",
}, p++);

addFormulaSlide(presentation, {
  title: "Support minimizes prevalence before applying joint survival",
  section: "Problem III · Prevalence-robust supported performance",
  accent: C.blue,
  latexLines: [
    String.raw`Z_j=\inf_{π∈Π_{∗}}\operatorname{CNAP}_π(D_j)`,
    String.raw`S_{K,Π_{∗}}^{\mathrm{AP}}(\mathrm{R})=\operatorname{E}_{\mathrm{R}}\left[\min_{j≤K}Z_j\right]`,
  ],
  formulaSizes: [30, 30],
  meaning: "Within every replication, find its weakest target population; then require all K replications to retain the level.",
  why: "At K=2, the result is expected worst-prevalence performance minus half the expected disagreement between replications.",
  source: "Subsection “Prevalence-robust supported performance”",
}, p++);

addTwoColumnSlide(presentation, {
  title: "What fails if prevalence minimization occurs after replication?",
  section: "Problem III · Why operator order matters",
  accent: C.coral,
  leftFill: C.coralSoft,
  rightFill: C.blueSoft,
  leftKicker: "INTENDED CLAIM",
  leftHead: "E[minj  infπ CNAPπ(Dj)]",
  leftLatex: String.raw`\operatorname{E}\left[\min_j\inf_π\operatorname{CNAP}_π(D_j)\right]`,
  leftHeadSize: 29,
  leftBody: "Each replication reveals the population that defeats it. Replication 1 may fail at 1%; replication 2 at 40%.",
  rightKicker: "WEAKER CLAIM",
  rightHead: "infπ E[minj CNAPπ(Dj)]",
  rightLatex: String.raw`\inf_π\operatorname{E}\left[\min_j\operatorname{CNAP}_π(D_j)\right]`,
  rightHeadSize: 29,
  rightBody: "All replications are challenged at the same prevalence before the population minimum is taken.",
  source: "Subsection “Why the prevalence search occurs inside each replication”",
}, p++);

addThreeColumnSlide(presentation, {
  title: "Low support can reflect shifting worst cases or unstable replications",
  section: "Problem III · Why operator order matters",
  accent: C.coral,
  items: [
    { kicker: "START", head: "Best worst-population mean", body: "The lowest expected profile value over the declared prevalence set.", fill: C.gray },
    { kicker: "− CPREV", head: "Moving limiting prevalence", body: "The weakest population changes across replications.", fill: C.coralSoft },
    { kicker: "− CREP", head: "Replication disagreement", body: "Retained levels vary; at K=2 this is ½E|Z₁−Z₂|.", fill: C.blueSoft },
  ],
  bottomLatex: String.raw`S_{K,Π_{∗}}^{\mathrm{AP}}=\inf_{π∈Π_{∗}}\operatorname{E}_{\mathrm{R}}[\operatorname{CNAP}_π]-C_{\mathrm{prev}}-C_{\mathrm{rep}}`,
  bottomSize: 22,
  source: "Subsection “Why the prevalence search occurs inside each replication”",
}, p++);

addFormulaSlide(presentation, {
  title: "Expanding the challenge set cannot increase raw support",
  section: "Problem III · Severity and its limits",
  accent: C.gold,
  formulaFill: C.goldSoft,
  latexLines: [
    String.raw`Π_1⊆Π_2,\,K_1≤K_2`,
    String.raw`S_{K_2,Π_2}^{\mathrm{AP}}(\mathrm{R})≤S_{K_1,Π_1}^{\mathrm{AP}}(\mathrm{R})`,
  ],
  formulaSizes: [31, 28],
  meaning: "Broader population coverage or more independent chances for defeat can only keep or lower support.",
  why: "Arbitrary regimes are not ordered this way: changing what is redrawn changes the scientific claim, not merely its index set.",
  source: "Subsection “Severity and its limits”",
}, p++);

addProcessSlide(presentation, {
  title: "How can one evaluation estimate a replication-level target?",
  section: "Estimating support · The fixed-model bootstrap",
  accent: C.blue,
  lead: "Observed evaluation units must be the mechanism the regime redraws.",
  steps: [
    { head: "Resample within class", body: "Draw planned positive and negative counts with replacement.", fill: C.blueSoft },
    { head: "Recombine units", body: "Keep each unit’s score and outcome together.", fill: C.gray },
    { head: "Evaluate all π", body: "Use the same resampled units across the full prevalence set.", fill: C.tealSoft },
    { head: "Record the retained level", body: "Store the lowest CNAP over the declared prevalence set.", fill: C.goldSoft },
  ],
  source: "Subsection “The fixed-model bootstrap”",
}, p++);

addFormulaSlide(presentation, {
  title: "A finite grid can miss the worst prevalence in a continuous set",
  section: "Estimating support · Intervals, grids, and richer regimes",
  accent: C.teal,
  formulaFill: C.tealSoft,
  latexLines: [
    String.raw`Π_{∗}=\{π_1,…,π_G\}\,\text{ finite challenge}`,
    String.raw`Π_{∗}=[π_L,π_U]\,\text{ continuous challenge}`,
  ],
  formulaSizes: [30, 30],
  formulaSize: 35,
  meaning: "A coarse grid can miss an interior minimum and inflate the result.",
  why: "Outer resampling repeats the complete inner procedure. Richer regimes require data or a hierarchy that actually represents them.",
  source: "Subsection “Intervals, grids, and richer regimes”",
}, p++);

addTwoColumnSlide(presentation, {
  title: "Why can random ranking exceed zero in a finite design?",
  section: "Problem IV · The displacement",
  accent: C.coral,
  leftFill: C.gray,
  rightFill: C.coralSoft,
  leftKicker: "ONE POSITIVE + ONE NEGATIVE",
  leftHead: "Random order gives CNAP = 1 or 0",
  leftHeadSize: 29,
  leftBody: "Each ordering occurs with probability ½. Mean CNAP = ½.",
  rightKicker: "TWO INDEPENDENT REPLICATIONS",
  rightHead: "E[min(Z₁,Z₂)] = ¼",
  rightLatex: String.raw`\operatorname{E}[\min(Z_1,Z_2)]=\frac14`,
  rightHeadSize: 31,
  rightBody: "Pure noise returns +0.25 on a population scale whose zero denotes random ranking.",
  source: "Subsection “The displacement”",
}, p++);

addFormulaSlide(presentation, {
  title: "Permutation supplies a finite-design reference for one model",
  section: "Problem IV · A design-relative report",
  accent: C.teal,
  formulaFill: C.tealSoft,
  latex: String.raw`S_{\mathrm{est}}^{\mathrm{cal}}=\frac{S_{\mathrm{est}}-S_{\mathrm{est}}^{\mathrm{null}}}{1-S_{\mathrm{est}}^{\mathrm{null}}}`,
  formulaSize: 39,
  formulaCaption: "Condition on the score vector, tie structure, units, and class counts",
  meaning: "Permutation estimates the finite-design anchor for this fixed score vector.",
  why: "The calibrated value is conditional on the realized score vector, tie structure, units, and class counts.",
  source: "Subsection “A design-relative report for one model”",
}, p++);

addTwoColumnSlide(presentation, {
  title: "Can two separately calibrated reports decide model replacement?",
  section: "Problem V · Why marginal reports do not compose",
  accent: C.coral,
  leftFill: C.coralSoft,
  rightFill: C.tealSoft,
  leftKicker: "MARGINAL SUBTRACTION",
  leftHead: "SK(Aworst) − SK(Bworst)",
  leftLatex: String.raw`S_K(A_{Π_{∗}})-S_K(B_{Π_{∗}})`,
  leftHeadSize: 28,
  leftBody: "A and B may meet different worst prevalences and different worst replications.",
  rightKicker: "PAIRED CLAIM",
  rightHead: "SK(infπ{Aπ − Bπ})",
  rightLatex: String.raw`S_K\left(\inf_{π∈Π_{∗}}\{A_π-B_π\}\right)`,
  rightHeadSize: 28,
  rightBody: "Compare on the same units, at the same prevalence, before any population or replication minimum.",
  source: "Subsection “Why marginal reports do not compose”",
}, p++);

addTwoColumnSlide(presentation, {
  title: "Even paired comparison can reward arbitrary tie-breaking",
  section: "Problem V · Why ordinary pairing is still not neutral",
  accent: C.coral,
  leftFill: C.blueSoft,
  rightFill: C.coralSoft,
  leftKicker: "MODEL A",
  leftHead: "Four distinct scores",
  leftBody: "With 2 positive and 2 negative labels assigned at random, its expected CNAP advantage is about 0.361.",
  rightKicker: "MODEL B",
  rightHead: "One fully tied score",
  rightBody: "Neither score vector is associated with outcomes—yet A receives credit merely for breaking ties.",
  source: "Subsection “Why ordinary pairing is still not neutral”",
}, p++);

addFormulaSlide(presentation, {
  title: "Tie averaging removes arbitrary within-tie advantage",
  section: "Problem V · Tie-averaged average precision",
  accent: C.blue,
  latex: String.raw`\operatorname{AP}^{\mathrm{TA}}_π(D)=\frac{1}{|\mathrm{O}(D)|}\sum_{o∈\mathrm{O}(D)}\operatorname{AP}_π(D_o)`,
  formulaSize: 34,
  formulaCaption: "Average over all strict rankings consistent with the score blocks",
  meaning: "Average AP over every within-tie ordering consistent with the observed score blocks.",
  why: "Outcome-relevant refinement can still improve AP; granularity alone receives no expected credit.",
  source: "Subsection “Tie-averaged average precision”",
}, p++);

addFormulaSlide(presentation, {
  title: "Pair model differences before prevalence and replication challenges",
  section: "Problem V · The paired supported advantage",
  accent: C.teal,
  formulaFill: C.tealSoft,
  latexLines: [
    String.raw`Δ_π^{\mathrm{TA}}(D)=\operatorname{CNAP}^{\mathrm{TA}}_{A,π}(D)-\operatorname{CNAP}^{\mathrm{TA}}_{B,π}(D)`,
    String.raw`θ_{A:B}=\operatorname{E}_{\mathrm{R}}\left[\min_{j≤K}\inf_{π∈Π_{∗}}Δ_{π,j}^{\mathrm{TA}}\right]`,
  ],
  formulaSizes: [25, 28],
  meaning: "Same units → tie-neutral difference → prevalence challenge → replication challenge.",
  why: "Estimation preserves pairing at every layer. Neither marginal model score is an intermediate input.",
  source: "Subsection “The paired supported advantage”",
}, p++);

addThreeColumnSlide(presentation, {
  title: "A zero-reference guarantee requires exchangeable fixed-count labels",
  section: "Problem V · Reference behavior and design scope",
  accent: C.gold,
  items: [
    { kicker: "REFERENCE LAW", kickerWidth: 150, head: "All fixed-count label vectors are equally likely", body: "Condition on both score vectors and n+.", fill: C.goldSoft },
    { kicker: "CONSEQUENCE", kickerWidth: 150, head: "Expected paired profile is zero", body: "Tie structure no longer changes the expected AP anchor.", fill: C.tealSoft },
    { kicker: "BOUND", head: "Supported advantage ≤ 0 under noise", body: "The infimum and minimum may push the reference value below zero.", fill: C.gray },
  ],
  bottom: "Matched, clustered, or heterogeneous-risk designs may require a design-specific claim.",
  source: "Subsection “Reference behavior and design scope”",
}, p++);

addFormulaSlide(presentation, {
  title: "A valid replacement rule may return no verdict",
  section: "Problem V · No verdict and replacement",
  accent: C.coral,
  formulaFill: C.coralSoft,
  latex: String.raw`θ_{A:B}+θ_{B:A}≤0,\,\operatorname{LB}(θ_{A:B})>d`,
  formulaSize: 31,
  meaning: "Both orientations cannot show a positive supported advantage.",
  why: "If both are nonpositive, the declared challenges defeated superiority in either direction. The practical margin d belongs to the decision specification.",
  source: "Subsection “No verdict and replacement”",
}, p++);

addThreeColumnSlide(presentation, {
  title: "What does a supported-performance value actually describe?",
  section: "Interpretation · What the number says",
  accent: C.blue,
  items: [
    { kicker: "Π★", head: "Where the claim had to hold", body: "The target prevalence set travels with the value.", fill: C.blueSoft },
    { kicker: "ℛ", head: "What could expose failure", body: "Fixed model, new environment, or repeated development.", fill: C.tealSoft },
    { kicker: "K", head: "How many failures were possible", body: "Independent challenges required to retain the level jointly.", fill: C.goldSoft },
  ],
  bottom: "It is a supported magnitude—not a confidence level or a probability of truth.",
  source: "Subsection “What the number says”",
}, p++);

addThreeColumnSlide(presentation, {
  title: "A complete report separates estimand, decision, and evidence",
  section: "Interpretation · What should be reported",
  accent: C.teal,
  items: [
    { kicker: "ESTIMAND", kickerWidth: 150, head: "Supported quantity + assessment", body: "Raw value; Π★ and its basis; ℛ; K; replication counts and resampling unit.", fill: C.tealSoft },
    { kicker: "DECISION RULE", kickerWidth: 150, head: "Pairing + practical margin", body: "Orientation, tie averaging, exchangeability argument, d, and reversed orientation.", fill: C.blueSoft },
    { kicker: "EVIDENCE", kickerWidth: 150, head: "Diagnostics + uncertainty", body: "Profiles, minimizing prevalences, costs, computation, Monte Carlo error, and validated outer bound.", fill: C.gray },
  ],
  source: "Subsection “What should be reported”",
}, p++);

addTwoColumnSlide(presentation, {
  title: "Sparse positives coarsen the supported-performance profile",
  section: "Interpretation · Sparse outcomes and uncertainty",
  accent: C.coral,
  leftFill: C.coralSoft,
  rightFill: C.blueSoft,
  leftKicker: "WHAT CHANGES",
  leftHead: "Each positive makes one full recall step",
  leftHeadSize: 29,
  leftBody: "One case can reshape the complete profile, its lower tail, and the minimizing prevalence.",
  rightKicker: "WHAT DOES NOT HELP",
  rightHead: "More bootstrap draws do not create more positives",
  rightHeadSize: 28,
  rightBody: "Show the replication distribution and minimizers. Decimal precision is not evidential precision.",
  source: "Subsection “Sparse outcomes and uncertainty”",
}, p++);

addThreeColumnSlide(presentation, {
  title: "What conclusions remain outside the method’s scope?",
  section: "Interpretation · Inferences the result does not license",
  accent: C.coral,
  items: [
    { kicker: "TRANSPORT", kickerWidth: 150, head: "Beyond prior shift", body: "Reweighting does not protect against within-class behavior changes.", fill: C.coralSoft },
    { kicker: "DESIGN", head: "Pairing alone removes chance", body: "Exchangeable labels are also required for the reference guarantee.", fill: C.goldSoft },
    { kicker: "DEPLOYMENT", kickerWidth: 150, head: "Good ranking implies intervention benefit", body: "Passive evaluation does not identify effects of acting on the ranking.", fill: C.blueSoft },
  ],
  bottom: "Nor does a common normalized scale create a universal leaderboard.",
  source: "Subsection “Inferences the result does not license”",
}, p++);

addFormulaSlide(presentation, {
  title: "The final estimand combines worst-prevalence and joint survival",
  section: "Conclusion",
  accent: C.blue,
  latexLines: [
    String.raw`Z_j=\inf_{π∈Π_{∗}}\operatorname{CNAP}_π(D_j)`,
    String.raw`S_{K,Π_{∗}}^{\mathrm{AP}}=\operatorname{E}_{\mathrm{R}}\left[\min_{j≤K}Z_j\right]`,
  ],
  formulaSizes: [30, 31],
  formulaCaption: "population challenge inside each replication · corroboration across replications",
  meaning: "The estimand is indexed by the target prevalence set, replication law, and K.",
  why: "For replacement, paired tie-averaged differences are compared with a practical margin through a validated outer bound.",
  source: "Conclusion",
}, p++);

addDividerSlide(presentation, {
  title: "Technical appendix:\ndefinitions, proofs, and computation",
  subtitle: "Mathematical properties, design conditions, and implementation details.",
  accent: "#7DB4FF",
  source: "Appendix overview",
}, p++);

addFormulaSlide(presentation, {
  title: "Prior-standardized AP is weighted stepwise precision",
  section: "Appendix A · Definition and weighted implementation",
  accent: C.blue,
  latexLines: [
    String.raw`p_{π,h}=\frac{π r_h}{π r_h+(1-π)f_h}`,
    String.raw`\operatorname{AP}_π(D)=\sum_{h=1}^{H}(r_h-r_{h-1})p_{π,h}`,
  ],
  formulaSizes: [31, 29],
  meaning: "Each recall increment is weighted by precision in the target population.",
  why: "Equivalent weights are π/n+ for every positive and (1−π)/n− for every negative. Score ties enter as threshold blocks.",
  source: "Appendix subsection “Definition and weighted implementation”",
}, p++);

addFormulaSlide(presentation, {
  title: "The CNAP minimum may lie inside the prevalence interval",
  section: "Appendix A · Behavior across prevalence",
  accent: C.teal,
  formulaFill: C.tealSoft,
  latexLines: [
    String.raw`q_h(π)=\frac{π(r_h-f_h)}{π r_h+(1-π)f_h}`,
    String.raw`q_h'(π)=\frac{f_h(r_h-f_h)}{\{π r_h+(1-π)f_h\}^2}`,
  ],
  formulaSizes: [29, 27],
  meaning: "If the true-positive rate is at least the false-positive rate at every contributing threshold, the profile is nondecreasing.",
  why: "Then the interval minimum is at πL. Thresholds crossing the ROC diagonal—or paired differences—can create interior extrema.",
  source: "Appendix subsection “Behavior across prevalence”",
}, p++);

addFormulaSlide(presentation, {
  title: "Expected minimum equals integrated joint survival",
  section: "Appendix B · Survival-function representation",
  accent: C.coral,
  formulaFill: C.coralSoft,
  latex: String.raw`S_K(T)=L+\int_L^U\operatorname{P}(T>s)^K\,ds`,
  formulaSize: 38,
  meaning: "At every level s, P(T>s)K is the chance that all K independent replications retain it.",
  why: "For robust CNAP, T>s means the profile exceeds s at every π in Π★.",
  source: "Appendix subsection “Survival-function representation”",
}, p++);

addProofSlide(presentation, {
  title: "Expectation creates the operator-order gap",
  section: "Appendix B · Proof of the operator-order inequality",
  accent: C.teal,
  latex: String.raw`\min_j\inf_π X_π^{(j)}=\inf_π\min_j X_π^{(j)}`,
  steps: [
    { head: "Pathwise", body: "Minimum over replications and infimum over prevalence commute on each realized array." },
    { head: "Then average", body: "The expected profilewise minimum cannot exceed the minimum expected profile value." },
    { head: "Equality", body: "A common almost-sure minimizing prevalence removes the gap." },
  ],
  source: "Appendix subsection “Proof of the operator-order inequality”",
}, p++);

addProofSlide(presentation, {
  title: "Challenge monotonicity follows from pathwise set inclusion",
  section: "Appendix B · Proof of challenge monotonicity",
  accent: C.gold,
  formulaFill: C.goldSoft,
  latex: String.raw`\min_{j≤K_2}\inf_{π∈Π_2}X_π^{(j)}≤\min_{j≤K_1}\inf_{π∈Π_1}X_π^{(j)}`,
  steps: [
    { head: "Couple", body: "Draw K₂ replications once." },
    { head: "Reuse", body: "Use the first K₁ for the smaller assessment." },
    { head: "Contain", body: "The larger challenge set contains every index in the smaller one." },
  ],
  source: "Appendix subsection “Proof of challenge monotonicity”",
}, p++);

addFormulaSlide(presentation, {
  title: "Increasing K shifts support toward the lower tail",
  section: "Appendix B · Quantile representation and dispersion",
  accent: C.blue,
  latex: String.raw`S_K(T)=\int_0^1 Q(v)\,K(1-v)^{K-1}\,dv`,
  formulaSize: 36,
  meaning: "The minimum of K draws samples lower quantiles more heavily as K grows.",
  why: "At K=2: support = first L-moment − second L-moment. A mean-preserving spread cannot raise support.",
  source: "Appendix subsection “Quantile representation and dispersion”",
}, p++);

addFormulaSlide(presentation, {
  title: "Why does joint survival imply a power distortion?",
  section: "Appendix C · Why joint survival selects a power distortion",
  accent: C.coral,
  formulaFill: C.coralSoft,
  latexLines: [
    String.raw`g(u_1u_2)=g(u_1)g(u_2)\,\rightarrow\,g(u)=u^c`,
    String.raw`K\text{ replications}\,\rightarrow\,c=K`,
  ],
  formulaSizes: [28, 31],
  meaning: "Weights must compose multiplicatively when independent challenges are conjoined.",
  why: "This characterizes the family under explicit representation assumptions; it does not make quantiles or expected shortfall illegitimate.",
  source: "Appendix section “Why joint survival selects a power distortion”",
}, p++);

addProofSlide(presentation, {
  title: "Why must model differences be paired before subtraction?",
  section: "Appendix D · Why marginal subtraction overstates",
  accent: C.coral,
  formulaFill: C.coralSoft,
  latex: String.raw`S_K(U_{Π_{∗}})-S_K(V_{Π_{∗}})≥S_K(T_{Π_{∗}})`,
  steps: [
    { head: "Population step", body: "The worst paired difference cannot exceed the difference between separate worst profiles." },
    { head: "Replication step", body: "The minimum is superadditive on paired replications." },
    { head: "Conclusion", body: "Separate adverse cases lose the joint comparison." },
  ],
  source: "Appendix subsection “Why marginal subtraction overstates the paired advantage”",
}, p++);

addProofSlide(presentation, {
  title: "Tie refinement is neutral by the tower property",
  section: "Appendix D · Proof of refinement neutrality",
  accent: C.teal,
  latex: String.raw`\operatorname{E}\left[\operatorname{AP}^{\mathrm{TA}}_{A,π}\,|\,S_B,Y\right]=\operatorname{AP}^{\mathrm{TA}}_{B,π}`,
  steps: [
    { head: "Condition", body: "Hold the coarse score blocks and realized outcomes fixed." },
    { head: "Average", body: "A’s strict ranking is uniform over the orderings allowed by B’s ties." },
    { head: "Collapse", body: "Expected ordinary AP becomes B’s tie-averaged AP." },
  ],
  source: "Appendix subsection “Proof of refinement neutrality”",
}, p++);

addProofSlide(presentation, {
  title: "Exchangeable labels make the zero-reference law rank-invariant",
  section: "Appendix D · Rank invariance under exchangeable labels",
  accent: C.blue,
  latex: String.raw`\operatorname{E}_0[Δ^{\mathrm{TA}}_π]=0`,
  steps: [
    { head: "Uniform sequences", body: "Along any fixed strict ranking, fixed-count label sequences have the same law." },
    { head: "Average ties", body: "A convex combination of equal expectations keeps the same anchor." },
    { head: "Challenge", body: "Infimum and minimum can only move paired support to ≤ 0." },
  ],
  source: "Appendix subsection “Rank invariance under exchangeable labels”",
}, p++);

addTwoColumnSlide(presentation, {
  title: "Restricted label symmetry leaves multiple design orbits",
  section: "Appendix D · Why restricted label symmetry is insufficient",
  accent: C.coral,
  leftFill: C.coralSoft,
  rightFill: C.tealSoft,
  leftKicker: "MATCHED EXAMPLE",
  leftHead: "Two pairs, exactly one positive per pair",
  leftHeadSize: 28,
  leftBody: "Permitted permutations swap labels only within pairs. A score aligned with pairs carries design information.",
  rightKicker: "CONSEQUENCE",
  rightHead: "Tie averaging cannot cross design orbits",
  rightHeadSize: 28,
  rightBody: "Possible remedies change the claim: restrict model pairs, evaluate within strata, or standardize to another target population.",
  source: "Appendix subsection “Why restricted label symmetry is insufficient”",
}, p++);

addTwoColumnSlide(presentation, {
  title: "A null anchor cannot serve as an additive correction",
  section: "Appendix D · Why paired null centering fails",
  accent: C.coral,
  leftFill: C.blueSoft,
  rightFill: C.coralSoft,
  leftKicker: "INFORMATIVE DATA",
  leftHead: "Both models separate perfectly",
  leftBody: "Paired CNAP difference is zero in every stratified bootstrap replication.",
  rightKicker: "NULL-CENTERED RESULT",
  rightHead: "One orientation can falsely favor the coarser model",
  rightHeadSize: 26,
  rightBody: "The displacement is largest near no association and must vanish at the ceiling. It cannot be subtracted uniformly.",
  source: "Appendix subsection “Why paired null centering fails”",
}, p++);

addFormulaSlide(presentation, {
  title: "Reversing orientation turns a minimum into a maximum",
  section: "Appendix D · Proof of the orientation bound",
  accent: C.teal,
  formulaFill: C.tealSoft,
  latex: String.raw`S_K(T)+S_K(-T)=\operatorname{E}\left[\min_{j≤K}T_j-\max_{j≤K}T_j\right]≤0`,
  formulaSize: 34,
  meaning: "Reversing orientation converts the expected minimum into the negative expected maximum.",
  why: "At K=2, the negative sum is E|T₁−T₂|: the width of the no-verdict region.",
  source: "Appendix subsection “Proof of the orientation bound”",
}, p++);

addProcessSlide(presentation, {
  title: "How can tie-averaged AP avoid enumerating tie orders?",
  section: "Appendix E · Closed-form computation",
  accent: C.blue,
  lead: "For each score block, average the precision contribution of one representative positive.",
  steps: [
    { head: "Choose position k", body: "Within a block of size mj, k is uniform.", fill: C.blueSoft },
    { head: "Count earlier positives t", body: "t follows a hypergeometric law.", fill: C.tealSoft },
    { head: "Compute rjkt and fjkt", body: "Add preceding blocks and the within-block counts.", fill: C.gray },
    { head: "Average and sum", body: "Reuse combinatorial weights across π values.", fill: C.goldSoft },
  ],
  source: "Appendix section “Closed-form computation of tie-averaged AP”",
}, p++);

addFormulaSlide(presentation, {
  title: "Sorting yields the complete order-K support estimator",
  section: "Appendix F · General support order",
  accent: C.blue,
  latex: String.raw`S_{K,\mathrm{est}}=\frac{1}{\mathrm C(B,K)}\sum_{i=1}^{B-K+1}\mathrm C(B-i,K-1)\,z_{(i)}`,
  formulaSize: 32,
  meaning: "z(i) is the minimum in every K-subset that contains it and K−1 larger effects.",
  why: "Sorting avoids enumerating all C(B,K) subsets. For K=2, the weight is proportional to B−i.",
  source: "Appendix subsection “General support order”",
}, p++);

addFormulaSlide(presentation, {
  title: "Leave-one-replication projections quantify Monte Carlo error",
  section: "Appendix F · Monte Carlo error",
  accent: C.teal,
  formulaFill: C.tealSoft,
  latex: String.raw`\operatorname{SE}_{\mathrm{MC}}=\frac{2}{\sqrt B}\operatorname{sd}_i\left\{h^{\mathrm{est}}_{1,-i}(z_i)\right\}`,
  formulaSize: 36,
  meaning: "Measure how much each computational replication influences its average pairwise minimum.",
  why: "Prefix sums compute all projections after sorting. Conditional permutation adds a second Monte Carlo component governed by M.",
  source: "Appendix subsection “Monte Carlo error”",
}, p++);

addThreeColumnSlide(presentation, {
  title: "Efficient prevalence evaluation reuses threshold structure",
  section: "Appendix F · Prevalence evaluation",
  accent: C.gold,
  items: [
    { kicker: "FINITE SET", kickerWidth: 150, head: "Reuse threshold counts", body: "Ordinary block AP costs O(BGH) for B bootstraps, G prevalences, H thresholds.", fill: C.goldSoft },
    { kicker: "PAIRED TIES", kickerWidth: 150, head: "Reuse block combinatorics", body: "Only the prevalence-dependent precision term changes.", fill: C.tealSoft },
    { kicker: "INTERVAL", kickerWidth: 150, head: "Optimize one dimension", body: "Check endpoints and stationary points; validate against a dense grid.", fill: C.blueSoft },
  ],
  source: "Appendix subsection “Prevalence evaluation”",
}, p++);

addProcessSlide(presentation, {
  title: "Outer confidence bounds must repeat the full inner procedure",
  section: "Appendix F · Outer bounds",
  accent: C.coral,
  lead: "The outer layer asks how well the observed evaluation identifies the supported target.",
  steps: [
    { head: "Outer sample", body: "Resample the observed evaluation within class.", fill: C.coralSoft },
    { head: "Inner support", body: "Repeat bootstrap, prevalence search, and joint-survival estimator.", fill: C.blueSoft },
    { head: "Preserve design", body: "Keep model pairs attached; recompute single-model anchors.", fill: C.tealSoft },
    { head: "Validate the bound", body: "Nonsmooth minima can break nominal coverage.", fill: C.goldSoft },
  ],
  source: "Appendix subsection “Outer bounds”",
}, p++);

if (p !== 51) {
  throw new Error(`Expected 50 slides, but next page number is ${p}`);
}

await fs.mkdir(OUT_DIR, { recursive: true });
for (const [index, slide] of presentation.slides.items.entries()) {
  const stem = `slide-${String(index + 1).padStart(2, "0")}`;
  const png = await presentation.export({ slide, format: "png", scale: 1 });
  await writeBlob(`${OUT_DIR}/${stem}.png`, png);
  const layout = await slide.export({ format: "layout" });
  await fs.writeFile(`${OUT_DIR}/${stem}.layout.json`, await layout.text());
}

const montage = await presentation.export({ format: "webp", montage: true, scale: 0.35 });
await writeBlob(`${OUT_DIR}/deck-montage.webp`, montage);

const pptx = await PresentationFile.exportPptx(presentation);
await pptx.save(FINAL_PPTX);

console.log(`Wrote ${presentation.slides.items.length} slides to ${FINAL_PPTX}`);
