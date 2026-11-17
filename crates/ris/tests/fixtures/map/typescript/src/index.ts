import { save } from "./store";
import { Frame } from "./wire.js";
export { ui } from "./ui";

export function run(f: Frame): void {
  save(f);
}
