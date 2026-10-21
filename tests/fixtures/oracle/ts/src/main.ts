import { add as plus, Calc } from "@lib/math";
import { upper } from "@util";
import type { Calc as CalcT } from "./lib/math";
import express from "express";
import * as fs from "node:fs";

const c = new Calc(1);
c.compute();
plus(1, 2);
upper("x");
// mul(1, 2)
const s = "internal()";
export default function main() {
  return fs.existsSync(".");
}
