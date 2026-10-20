import { expect, it } from "vitest";
import { LANGUAGES } from "./languages";

it("offers only explicit English and Dutch", () => {
  expect(LANGUAGES.map((language) => language.value).sort()).toEqual(["en", "nl"]);
});
