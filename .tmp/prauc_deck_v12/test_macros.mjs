import { mathFromLatex, mathToDisplayText } from "@oai/artifact-tool";
for (const latex of [
  "\\mathbb E_{\\mathcal R}",
  "\\mathbb{E}_{\\mathcal{R}}",
  "\\operatorname{CNAP}",
  "\\mathrm{CNAP}",
  "\\text{replace}",
  "\\quad",
  "\\Longrightarrow",
  "\\substack{a\\\\b}",
  "\\Rightarrow",
  "\\rightarrow",
  "\\,",
  "\\;",
  "\\!",
  "\\mathcal{O}",
  "\\mathrm{R}",
  "\\widetilde{x}",
  "\\widehat{x}",
  "\\hat{x}",
  "\\binom{B}{K}",
  "\\tilde{x}",
  "\\bar{x}",
]) {
  try {
    const m = mathFromLatex({ latex, displayMode: "block" });
    console.log(latex, "=>", mathToDisplayText(m));
  } catch (error) {
    console.log("ERR", latex, error.message);
  }
}
