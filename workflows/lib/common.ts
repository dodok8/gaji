export function addBuildGajiSteps(s) {
  return s
    .add({ name: "Build gaji", run: "cargo build --release" })
    .add({ name: "Generate types", run: "./target/release/gaji dev" });
}
