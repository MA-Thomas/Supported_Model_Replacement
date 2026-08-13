import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { equations } from "./render_equations.mjs";

const BUILD = "/Users/thomm15/Documents/Stats_Paper_AUPRC_extension/.deck_build_v12";
const INPUT = path.join(BUILD, "deck_with_equation_fallbacks.pptx");
const OUTPUT = "/Users/thomm15/Documents/Stats_Paper_AUPRC_extension/Stats_Paper_Extending_PRAUC_v12_Lab_Walkthrough.pptx";
const PACKAGE_DIR = path.join(BUILD, "native_math_package");
const EQUATION_DIR = path.join(BUILD, "equations");
const XSLT = "/Applications/Microsoft Word.app/Contents/Resources/mathml2omml.xsl";
const NAVY = "11253D";

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function escapeXml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;");
}

function mathmlToOmml(mathml, fontSizeHundredths) {
  const transformed = spawnSync("/usr/bin/xsltproc", [XSLT, "-"], {
    input: mathml,
    encoding: "utf8",
    maxBuffer: 8 * 1024 * 1024,
  });
  if (transformed.status !== 0) {
    throw new Error(`MathML → OMML failed: ${transformed.stderr}`);
  }

  let omml = transformed.stdout
    .replace(/^<\?xml[^>]*>\s*/u, "")
    .replace(
      /<m:oMathPara([^>]*)>/u,
      '<m:oMathPara$1><m:oMathParaPr><m:jc m:val="centerGroup"/></m:oMathParaPr>',
    );

  const runProperties =
    `<a:rPr lang="en-US" sz="${fontSizeHundredths}" dirty="0">` +
    `<a:solidFill><a:srgbClr val="${NAVY}"/></a:solidFill>` +
    '<a:latin typeface="Cambria Math"/><a:ea typeface="Cambria Math"/><a:cs typeface="Cambria Math"/>' +
    "</a:rPr>";
  omml = omml.replaceAll("<m:t>", `${runProperties}<m:t>`);
  return omml;
}

function estimateFontSizeHundredths(extCy, mathml) {
  const heightPoints = Number(extCy) / 12700;
  const hasFraction = mathml.includes("<mfrac>");
  const hasTable = mathml.includes("<mtable>");
  const hasLargeOperator =
    mathml.includes("<munderover>") ||
    mathml.includes("<munder>") ||
    mathml.includes("min");
  let points = heightPoints * (hasTable ? 0.29 : hasFraction || hasLargeOperator ? 0.43 : 0.58);
  points = Math.max(17, Math.min(points, 31));
  return Math.round(points * 100);
}

function makeEditableEquationChoice(picXml, equationId, omml) {
  const xfrmMatch = picXml.match(/<a:xfrm>[\s\S]*?<\/a:xfrm>/u);
  const extMatch = picXml.match(/<a:ext cx="(\d+)" cy="(\d+)"\/>/u);
  const idMatch = picXml.match(/<p:cNvPr id="(\d+)"[^>]*\/?>/u);
  if (!xfrmMatch || !extMatch || !idMatch) {
    throw new Error(`Could not recover picture geometry for ${equationId}`);
  }

  const shapeId = idMatch[1];
  const shapeName = `Editable equation — ${equationId}`;
  const fontSize = estimateFontSizeHundredths(extMatch[2], equations[equationId]);
  const nativeShape = `
    <p:sp>
      <p:nvSpPr>
        <p:cNvPr id="${shapeId}" name="${escapeXml(shapeName)}"/>
        <p:cNvSpPr txBox="1"><a:spLocks noGrp="1"/></p:cNvSpPr>
        <p:nvPr/>
      </p:nvSpPr>
      <p:spPr>
        ${xfrmMatch[0]}
        <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
        <a:noFill/>
        <a:ln w="0"><a:noFill/></a:ln>
      </p:spPr>
      <p:txBody>
        <a:bodyPr wrap="none" lIns="0" tIns="0" rIns="0" bIns="0" anchor="ctr">
          <a:normAutofit/>
        </a:bodyPr>
        <a:lstStyle/>
        <a:p>
          <a:pPr algn="ctr">
            <a:defRPr sz="${fontSize}" b="0" i="0">
              <a:solidFill><a:srgbClr val="${NAVY}"/></a:solidFill>
              <a:latin typeface="Cambria Math"/>
              <a:ea typeface="Cambria Math"/>
              <a:cs typeface="Cambria Math"/>
            </a:defRPr>
          </a:pPr>
          <a14:m>${omml}</a14:m>
          <a:endParaRPr lang="en-US" sz="${fontSize}" dirty="0"/>
        </a:p>
      </p:txBody>
    </p:sp>`;

  return `
    <mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
      <mc:Choice xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main" Requires="a14">
        ${nativeShape}
      </mc:Choice>
      <mc:Fallback>
        ${picXml}
      </mc:Fallback>
    </mc:AlternateContent>`;
}

async function main() {
  await fs.rm(PACKAGE_DIR, { recursive: true, force: true });
  await fs.mkdir(PACKAGE_DIR, { recursive: true });
  execFileSync("/usr/bin/unzip", ["-q", INPUT, "-d", PACKAGE_DIR]);

  const hashToEquation = new Map();
  for (const equationId of Object.keys(equations)) {
    const png = await fs.readFile(path.join(EQUATION_DIR, `${equationId}.png`));
    hashToEquation.set(sha256(png), equationId);
  }

  const mediaDir = path.join(PACKAGE_DIR, "ppt", "media");
  const mediaFiles = await fs.readdir(mediaDir);
  const mediaToEquation = new Map();
  for (const mediaFile of mediaFiles) {
    const bytes = await fs.readFile(path.join(mediaDir, mediaFile));
    const equationId = hashToEquation.get(sha256(bytes));
    if (equationId) mediaToEquation.set(mediaFile, equationId);
  }

  const ommlByEquation = new Map();
  const slidesDir = path.join(PACKAGE_DIR, "ppt", "slides");
  const slideFiles = (await fs.readdir(slidesDir))
    .filter((name) => /^slide\d+\.xml$/u.test(name))
    .sort((a, b) => Number(a.match(/\d+/u)[0]) - Number(b.match(/\d+/u)[0]));

  let replacementCount = 0;
  const foundEquations = new Set();
  for (const slideFile of slideFiles) {
    const slideNumber = slideFile.match(/\d+/u)[0];
    const slidePath = path.join(slidesDir, slideFile);
    const relsPath = path.join(slidesDir, "_rels", `${slideFile}.rels`);
    let slideXml = await fs.readFile(slidePath, "utf8");
    const relsXml = await fs.readFile(relsPath, "utf8");
    const relToMedia = new Map();
    for (const match of relsXml.matchAll(/<Relationship\b[^>]*\/>/gu)) {
      const relationship = match[0];
      const id = relationship.match(/\bId="([^"]+)"/u)?.[1];
      const type = relationship.match(/\bType="([^"]+)"/u)?.[1];
      const target = relationship.match(/\bTarget="([^"]+)"/u)?.[1];
      if (id && type?.endsWith("/image") && target) {
        relToMedia.set(id, path.basename(target));
      }
    }

    slideXml = slideXml.replace(/<p:pic>[\s\S]*?<\/p:pic>/gu, (picXml) => {
      const relMatch = picXml.match(/<a:blip\b[^>]*\br:embed="([^"]+)"/u);
      if (!relMatch) return picXml;
      const mediaFile = relToMedia.get(relMatch[1]);
      const equationId = mediaToEquation.get(mediaFile);
      if (!equationId) return picXml;

      const extMatch = picXml.match(/<a:ext cx="\d+" cy="(\d+)"\/>/u);
      if (!extMatch) throw new Error(`Missing equation extent on slide ${slideNumber}`);
      const fontSize = estimateFontSizeHundredths(extMatch[1], equations[equationId]);
      const cacheKey = `${equationId}:${fontSize}`;
      if (!ommlByEquation.has(cacheKey)) {
        ommlByEquation.set(cacheKey, mathmlToOmml(equations[equationId], fontSize));
      }

      replacementCount += 1;
      foundEquations.add(equationId);
      return makeEditableEquationChoice(picXml, equationId, ommlByEquation.get(cacheKey));
    });

    await fs.writeFile(slidePath, slideXml);
  }

  const missing = Object.keys(equations).filter((id) => !foundEquations.has(id));
  if (missing.length > 0) {
    throw new Error(`Equations not found in deck: ${missing.join(", ")}`);
  }

  await fs.rm(OUTPUT, { force: true });
  execFileSync("/usr/bin/zip", ["-q", "-r", OUTPUT, "."], { cwd: PACKAGE_DIR });
  console.log(`Inserted ${replacementCount} native Office Math equations into ${OUTPUT}`);
}

await main();
