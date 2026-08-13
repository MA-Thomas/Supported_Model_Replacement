import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "/Users/thomm15/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright/index.mjs";

const OUT_DIR = "/Users/thomm15/Documents/Stats_Paper_AUPRC_extension/.deck_build_v12/equations";

const m = (body) => `<math xmlns="http://www.w3.org/1998/Math/MathML" display="block">${body}</math>`;
const mi = (x) => `<mi>${x}</mi>`;
const mn = (x) => `<mn>${x}</mn>`;
const mo = (x) => `<mo>${x}</mo>`;
const mtext = (x) => `<mtext>${x}</mtext>`;
const row = (...xs) => `<mrow>${xs.join("")}</mrow>`;
const sub = (a, b) => `<msub>${a}${b}</msub>`;
const sup = (a, b) => `<msup>${a}${b}</msup>`;
const frac = (a, b) => `<mfrac>${a}${b}</mfrac>`;
const fenced = (x, open = "(", close = ")") => `<mrow><mo stretchy="true">${open}</mo>${x}<mo stretchy="true">${close}</mo></mrow>`;

export const equations = {
  s03_precision: m(row(mtext("Precision"), mo("="), frac(mi("TP"), row(mi("TP"), mo("+"), mi("FP"))))),
  s03_recall: m(row(mtext("Recall"), mo("="), frac(mi("TP"), row(mi("TP"), mo("+"), mi("FN"))))),
  s04_ap_mean: m(row(
    mi("AP"), mo("="),
    frac(row(mn("1.00"), mo("+"), mn("0.67"), mo("+"), mn("0.50")), mn("3")),
  )),
  s04_ap_result: m(row(mi("AP"), mo("="), mn("0.72"))),
  s05_rank1: m(row(frac(mn("1"), mn("1")), mo("="), mn("1.00"))),
  s05_rank3: m(row(frac(mn("2"), mn("3")), mo("="), mn("0.67"))),
  s05_rank6: m(row(frac(mn("3"), mn("6")), mo("="), mn("0.50"))),
  s06_rates: m(row(mi("TPR"), mo("="), mn("80"), mo("%"), mo(","), mi("FPR"), mo("="), mn("5"), mo("%"))),
  s07_r: m(row(
    mi("r"), fenced(mi("c")), mo("="),
    mi("P"), fenced(row(mi("S"), mo("≥"), mi("c"), mo("|"), mi("Y"), mo("="), mn("1"))),
  )),
  s07_f: m(row(
    mi("f"), fenced(mi("c")), mo("="),
    mi("P"), fenced(row(mi("S"), mo("≥"), mi("c"), mo("|"), mi("Y"), mo("="), mn("0"))),
  )),
  s07_pi: m(row(mi("π"), mo("="), mi("P"), fenced(row(mi("Y"), mo("="), mn("1"))))),
  s08_precision: m(row(
    sub(mi("p"), mi("π")), fenced(mi("c")), mo("="),
    frac(
      row(mi("π"), mi("r"), fenced(mi("c"))),
      row(
        mi("π"), mi("r"), fenced(mi("c")), mo("+"),
        fenced(row(mn("1"), mo("−"), mi("π"))), mi("f"), fenced(mi("c")),
      ),
    ),
  )),
  s09_invariance: m(row(
    sub(mi("P"), mi("obs")),
    fenced(row(mi("S"), mo("≥"), mi("c"), mo("|"), mi("Y"), mo("="), mi("y"))),
    mo("="),
    sub(mi("P"), mi("target")),
    fenced(row(mi("S"), mo("≥"), mi("c"), mo("|"), mi("Y"), mo("="), mi("y"))),
  )),
  s10_chance: m(row(sub(mi("AP"), mi("π")), mo("="), mi("π"))),
  s10_cnap: m(row(
    sub(mi("CNAP"), mi("π")), mo("="),
    frac(
      row(sub(mi("AP"), mi("π")), mo("−"), mi("π")),
      row(mn("1"), mo("−"), mi("π")),
    ),
  )),
  s13_assessment: m(row(mi("𝒜"), mo("="), fenced(row(mi("Π"), mo(","), mi("ℛ"), mo(","), mi("K"))))),
  s17_support: m(row(
    sub(mi("S"), mi("K")), fenced(row(mi("T"), mo(";"), mi("ℛ"))), mo("="),
    sub(mi("E"), mi("ℛ")),
    fenced(row(
      `<msub><mo>min</mo><mrow><mn>1</mn><mo>≤</mo><mi>j</mi><mo>≤</mo><mi>K</mi></mrow></msub>`,
      sub(mi("T"), mi("j")),
    ), "[", "]"),
  )),
  s18_support2: m(row(
    sub(mi("S"), mn("2")), fenced(row(mi("T"), mo(";"), mi("ℛ"))), mo("="),
    mi("E"), fenced(mi("T"), "[", "]"), mo("−"),
    frac(mn("1"), mn("2")),
    mi("E"), fenced(row(
      mo("|"), sub(mi("T"), mn("1")), mo("−"), sub(mi("T"), mn("2")), mo("|"),
    ), "[", "]"),
  )),
  s22_decomposition: m(row(
    mi("S"), mo("="),
    `<munder><mo>inf</mo><mi>π</mi></munder>`,
    mi("E"), fenced(sub(mi("CNAP"), mi("π")), "[", "]"),
    mo("−"), sub(mi("C"), mi("prev")),
    mo("−"), sub(mi("C"), mi("rep")),
  )),
  s24_order1: m(row(mi("AP"), mo("="), mn("1"), mo(","), mi("CNAP"), mo("="), mn("1"))),
  s24_order2: m(row(mi("AP"), mo("="), frac(mn("1"), mn("2")), mo(","), mi("CNAP"), mo("="), mn("0"))),
  s25_calibration: m(row(
    sub(mi("S"), mi("cal")), mo("="),
    frac(
      row(sub(mi("S"), mi("raw")), mo("−"), sub(mi("S"), mi("null"))),
      row(mn("1"), mo("−"), sub(mi("S"), mi("null"))),
    ),
  )),
  s26_bad_subtraction: m(row(mn("0.30"), mo("−"), mn("0.25"), mo("="), mn("0.05"))),
  s27_a: m(row(mi("E"), fenced(mi("AP"), "[", "]"), mo("="), frac(mn("49"), mn("72")))),
  s27_b: m(row(mi("AP"), mo("="), frac(mn("1"), mn("2")))),
  s27_bias: m(row(
    mi("E"), fenced(row(
      sub(mi("CNAP"), mi("A")), mo("−"), sub(mi("CNAP"), mi("B")),
    ), "[", "]"),
    mo("≈"), mn("0.361"),
  )),
  s28_ap1: m(row(mi("AP"), mo("="), mn("1.00"))),
  s28_ap2: m(row(mi("AP"), mo("="), mn("0.83"))),
  s28_ap3: m(row(mi("AP"), mo("="), mn("0.58"))),
  s29_pair: m(`<mtable rowspacing="0.2em" columnalign="center">
    <mtr><mtd>${row(sub(mi("Δ"), mi("π")), mo("="))}</mtd></mtr>
    <mtr><mtd>${row(
      sub(sub(mi("CNAP"), mi("A")), mi("π")), mo("−"),
      sub(sub(mi("CNAP"), mi("B")), mi("π")),
    )}</mtd></mtr>
  </mtable>`),
  s29_population: m(row(
    mi("δ"), mo("="),
    `<munder><mo>inf</mo><mrow><mi>π</mi><mo>∈</mo><mi>Π</mi></mrow></munder>`,
    sub(mi("Δ"), mi("π")),
  )),
  s29_replication: m(row(
    `<msub><mo>min</mo><mrow><mn>1</mn><mo>≤</mo><mi>j</mi><mo>≤</mo><mi>K</mi></mrow></msub>`,
    sub(mi("δ"), mi("j")),
  )),
  s29_expectation: m(`<mtable rowspacing="0.2em" columnalign="center">
    <mtr><mtd>${row(sub(mi("θ"), row(mi("A"), mo(":"), mi("B"))), mo("="))}</mtd></mtr>
    <mtr><mtd>${row(
      sub(mi("E"), mi("ℛ")),
      fenced(row(
        `<msub><mo>min</mo><mrow><mn>1</mn><mo>≤</mo><mi>j</mi><mo>≤</mo><mi>K</mi></mrow></msub>`,
        sub(mi("δ"), mi("j")),
      ), "[", "]"),
    )}</mtd></mtr>
  </mtable>`),
  s31_constraint: m(row(
    sub(mi("θ"), row(mi("A"), mo(":"), mi("B"))), mo("+"),
    sub(mi("θ"), row(mi("B"), mo(":"), mi("A"))), mo("≤"), mn("0"),
  )),
  s31_left: m(row(sub(mi("θ"), row(mi("A"), mo(":"), mi("B"))), mo("="), mo("−"), mn("0.02"))),
  s31_right: m(row(mo("−"), sub(mi("θ"), row(mi("B"), mo(":"), mi("A"))), mo("="), mn("0.03"))),
  s35_ap: m(row(
    sub(mi("AP"), mi("π")), fenced(mi("D")), mo("="),
    `<munderover><mo>∑</mo><mi>h</mi><mtext></mtext></munderover>`,
    fenced(row(sub(mi("r"), mi("h")), mo("−"), sub(mi("r"), row(mi("h"), mo("−"), mn("1"))))),
    frac(
      row(mi("π"), sub(mi("r"), mi("h"))),
      row(
        mi("π"), sub(mi("r"), mi("h")), mo("+"),
        fenced(row(mn("1"), mo("−"), mi("π"))), sub(mi("f"), mi("h")),
      ),
    ),
  )),
  s35_recall: m(row(sub(mi("r"), mi("h")), mo("−"), sub(mi("r"), row(mi("h"), mo("−"), mn("1"))))),
  s35_positive: m(row(mi("π"), sub(mi("r"), mi("h")))),
  s35_negative: m(row(fenced(row(mn("1"), mo("−"), mi("π"))), sub(mi("f"), mi("h")))),
  s37_estimator: m(row(
    `<mover><msub><mi>S</mi><mn>2</mn></msub><mo>^</mo></mover>`, mo("="),
    frac(mn("2"), row(mi("B"), fenced(row(mi("B"), mo("−"), mn("1"))))),
    `<munderover><mo>∑</mo><mrow><mi>i</mi><mo>=</mo><mn>1</mn></mrow><mrow><mi>B</mi><mo>−</mo><mn>1</mn></mrow></munderover>`,
    fenced(row(mi("B"), mo("−"), mi("i"))),
    sub(mi("z"), fenced(mi("i"))),
  )),
  s38_cost: m(row(mi("O"), fenced(row(`<munderover><mo>∑</mo><mi>j</mi><mtext></mtext></munderover>`, sup(sub(mi("m"), mi("j")), mn("2")))))),
};

async function renderEquations() {
  await fs.mkdir(OUT_DIR, { recursive: true });

  const browser = await chromium.launch({
    headless: true,
    executablePath: "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  });
  const page = await browser.newPage({
    viewport: { width: 1800, height: 500 },
    deviceScaleFactor: 3,
  });

  for (const [id, mathml] of Object.entries(equations)) {
    await page.setContent(`<!doctype html>
      <html>
        <head>
          <style>
            html, body { margin: 0; padding: 0; background: transparent; }
            body { display: inline-flex; align-items: center; justify-content: center; }
            #formula {
              display: inline-flex;
              align-items: center;
              justify-content: center;
              padding: 8px 12px;
              color: #11253D;
              font-family: "STIX Two Math", "Cambria Math", "Times New Roman", serif;
              font-size: 58px;
              font-weight: 600;
              line-height: 1;
              white-space: nowrap;
            }
            math { font-family: "STIX Two Math", "Cambria Math", "Times New Roman", serif; }
            mi { font-style: italic; }
            mtext { font-style: normal; }
          </style>
        </head>
        <body><div id="formula">${mathml}</div></body>
      </html>`);
    const el = page.locator("#formula");
    await el.screenshot({
      path: path.join(OUT_DIR, `${id}.png`),
      omitBackground: true,
    });
  }

  await browser.close();
  console.log(`Rendered ${Object.keys(equations).length} equations to ${OUT_DIR}`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  await renderEquations();
}
