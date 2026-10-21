export async function load() {
  const m = await import("./lib/math");
  return m.mul(2, 3);
}
