import { test } from "node:test";
import assert from "node:assert/strict";
import { bumpVersion, stampNextRelease } from "./stamp-release.mjs";

test("bumps each level and resets the lower ones", () => {
  assert.equal(bumpVersion("0.70.3", "patch"), "0.70.4");
  assert.equal(bumpVersion("0.70.3", "minor"), "0.71.0");
  assert.equal(bumpVersion("0.70.3", "major"), "1.0.0");
});
test("rejects an unknown level or a non-plain version", () => {
  assert.throws(() => bumpVersion("0.70.3", "huge"), /Unknown bump level/);
  assert.throws(() => bumpVersion("0.70.3-beta", "patch"), /plain x\.y\.z/);
});

const entry = (version, date) => `  {\n    version: "${version}",\n    date: "${date}",\n    title: "T",\n  },\n`;

test("stamps the one next entry and leaves the others alone", () => {
  const src = entry("next", "") + entry("0.70.0", "2026-10-20");
  assert.equal(
    stampNextRelease(src, "0.70.1", "2026-10-21"),
    entry("0.70.1", "2026-10-21") + entry("0.70.0", "2026-10-20"),
  );
});
test("releases without notes when there is no next entry", () => {
  const src = entry("0.70.0", "2026-10-20");
  assert.equal(stampNextRelease(src, "0.70.1", "2026-10-21"), src);
});
test("refuses two next candidates", () => {
  assert.throws(() => stampNextRelease(entry("next", "") + entry("next", ""), "0.70.1", "2026-10-21"), /found 2/);
});
