import fs from "node:fs/promises";
import path from "node:path";
import { FileBlob, PresentationFile } from "@oai/artifact-tool";

const TMP_DIR = "/Users/thomm15/Documents/Stats_Paper_AUPRC_extension/.pptx-points.CAFors";
const STARTER_PPTX = path.join(TMP_DIR, "template-starter.pptx");
const FINAL_PPTX =
  "/Users/thomm15/Documents/Stats_Paper_AUPRC_extension/Stats_Paper_Extending_PRAUC_v12_Lab_Walkthrough_points_only.pptx";
const PREVIEW_DIR = path.join(TMP_DIR, "final-preview");
const LAYOUT_DIR = path.join(TMP_DIR, "final-layout");

async function writeBlob(filePath, blob) {
  await fs.writeFile(filePath, new Uint8Array(await blob.arrayBuffer()));
}

async function main() {
  await fs.mkdir(PREVIEW_DIR, { recursive: true });
  await fs.mkdir(LAYOUT_DIR, { recursive: true });

  const presentation = await PresentationFile.importPptx(await FileBlob.load(STARTER_PPTX));
  const before = await presentation.inspect({
    target: { id: "ch/1g3ihw3m", beforeLines: 2, afterLines: 2 },
    kind: "slide,chart",
    maxChars: 4000,
  });
  await fs.writeFile(path.join(TMP_DIR, "before-edit-inspect.txt"), before.ndjson, "utf8");

  const chart = presentation.resolve("ch/1g3ihw3m");
  chart.scatterOptions.style = "marker";
  chart.scatterOptions.varyColors = false;
  chart.series.getItemAt(0).smooth = false;

  const after = await presentation.inspect({
    target: { id: "ch/1g3ihw3m", beforeLines: 2, afterLines: 2 },
    kind: "slide,chart",
    maxChars: 4000,
  });
  await fs.writeFile(path.join(TMP_DIR, "after-edit-inspect.txt"), after.ndjson, "utf8");

  for (const [index, slide] of presentation.slides.items.entries()) {
    const stem = `final-slide-${String(index + 1).padStart(2, "0")}`;
    await writeBlob(
      path.join(PREVIEW_DIR, `${stem}.png`),
      await presentation.export({ slide, format: "png", scale: 1 }),
    );
    const layout = await slide.export({ format: "layout" });
    await fs.writeFile(path.join(LAYOUT_DIR, `${stem}.layout.json`), await layout.text(), "utf8");
  }

  await writeBlob(
    path.join(TMP_DIR, "final-montage.webp"),
    await presentation.export({ format: "webp", montage: true, scale: 1 }),
  );

  const pptx = await PresentationFile.exportPptx(presentation);
  await pptx.save(FINAL_PPTX);
  console.log(FINAL_PPTX);
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
