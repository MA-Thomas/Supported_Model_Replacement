import fs from "node:fs/promises";
import { Presentation, layers, text } from "@oai/artifact-tool";
try {
  const p = Presentation.create({ slideSize: { width: 1280, height: 720 } });
  const s = p.slides.add();
  s.compose(
    layers({ width: "fill", height: "fill" }, [
      text([{ latex: String.raw`\frac{\pi r(c)}{\pi r(c)+(1-\pi)f(c)}`, displayMode: "block" }], {
        name: "equation",
        position: { left: 100, top: 150 },
        width: 1080,
        height: 240,
        style: {
          fontSize: "40px",
          typeface: "Cambria Math",
          color: "#102436",
          alignment: "center",
          verticalAlignment: "middle",
          autoFit: "shrinkText",
          insets: { top: 0, right: 0, bottom: 0, left: 0 },
        },
      }),
    ]),
    { frame: { left: 0, top: 0, width: 1280, height: 720 }, baseUnit: 1 },
  );
  const png = await p.export({ slide: s, format: "png", scale: 1 });
  await fs.writeFile("test-math.png", new Uint8Array(await png.arrayBuffer()));
} catch (error) {
  console.error("SHORT_ERROR", error?.message, error?.cause?.message);
}
