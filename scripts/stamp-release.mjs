// Turn the merged state of main into a release: bump the version in the four
// files that carry it, and give the changelog entry written as version "next"
// its real version and date. Run by the Build macOS app workflow on a push to
// main; prints the new version.
//
//   node scripts/stamp-release.mjs <patch|minor|major> <YYYY-MM-DD>
import { readFileSync, writeFileSync } from "node:fs";

export function bumpVersion(current, level) {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(current);
  if (!m) throw new Error(`Not a plain x.y.z version: ${current}`);
  const [ma, mi, pa] = m.slice(1).map(Number);
  if (level === "major") return `${ma + 1}.0.0`;
  if (level === "minor") return `${ma}.${mi + 1}.0`;
  if (level === "patch") return `${ma}.${mi}.${pa + 1}`;
  throw new Error(`Unknown bump level: ${level}`);
}

// Exactly one entry may be "next": none means the release has no notes, and two
// would leave it ambiguous which one this release is.
export function stampNextRelease(source, version, date) {
  const re = /version: "next",(\s*)date: "[^"]*",/g;
  const count = source.match(re)?.length ?? 0;
  if (count !== 1) {
    throw new Error(
      `Expected exactly one release entry with version "next" in src/content/releases.ts, found ${count}. ` +
        `Add the release notes to the dev → main PR as version "next".`,
    );
  }
  return source.replace(re, `version: "${version}",$1date: "${date}",`);
}

// Exact-match replace (not a JSON rewrite) so each file keeps its formatting.
function replaceOnce(file, from, to) {
  const src = readFileSync(file, "utf8");
  if (!src.includes(from)) throw new Error(`${file}: "${from}" not found`);
  writeFileSync(file, src.replace(from, to));
}

function main() {
  const [level, date] = process.argv.slice(2);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date ?? "")) throw new Error("Usage: stamp-release.mjs <patch|minor|major> <YYYY-MM-DD>");
  const current = JSON.parse(readFileSync("package.json", "utf8")).version;
  const next = bumpVersion(current, level);

  const notes = "src/content/releases.ts";
  writeFileSync(notes, stampNextRelease(readFileSync(notes, "utf8"), next, date));
  replaceOnce("package.json", `"version": "${current}"`, `"version": "${next}"`);
  replaceOnce("src-tauri/tauri.conf.json", `"version": "${current}"`, `"version": "${next}"`);
  replaceOnce("src-tauri/Cargo.toml", `\nversion = "${current}"`, `\nversion = "${next}"`);
  replaceOnce("src-tauri/Cargo.lock", `name = "humla"\nversion = "${current}"`, `name = "humla"\nversion = "${next}"`);
  console.log(next);
}

if (process.argv[1]?.endsWith("stamp-release.mjs")) {
  try {
    main();
  } catch (error) {
    console.error(`error: ${error.message}`);
    process.exitCode = 1;
  }
}
