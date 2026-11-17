import http from "node:http";
import { Frame } from "./wire";

export function send(f: Frame): void {
  http.get(`/frames/${f}`);
}
