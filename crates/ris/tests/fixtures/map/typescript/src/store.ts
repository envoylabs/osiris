import fs from "node:fs";
import type { Frame } from "./wire";

export function save(f: Frame): void {
  fs.writeFileSync("frames", String(f));
}
