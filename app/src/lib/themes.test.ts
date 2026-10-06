// The Theme list in Settings, then Appearance: what it shows for the saved
// settings, what picking an entry saves, and which theme is on screen.
//
// Run: cd app && npx vitest run src/lib/themes.test.ts

import { describe, expect, it } from "vitest";
import { THEMES, choiceSettings, listedTheme, themeInUse } from "./themes";

describe("the Theme list", () => {
  it("shows System, or the theme that light or dark uses", () => {
    expect(listedTheme({ theme: "system", lightTheme: "marble", darkTheme: "graphite" })).toBe("system");
    expect(listedTheme({ theme: "light", lightTheme: "marble", darkTheme: "graphite" })).toBe("marble");
    expect(listedTheme({ theme: "dark", lightTheme: "marble", darkTheme: "graphite" })).toBe("graphite");
    expect(listedTheme({ theme: "light" })).toBe("limestone");
    expect(listedTheme({ theme: "dark" })).toBe("slate");
  });

  it("shows the default for an id it does not know or a wrong value", () => {
    for (const bad of ["sandstone", "Marble", "slate", 5, null, ["marble"], { id: "marble" }]) {
      expect(listedTheme({ theme: "light", lightTheme: bad })).toBe("limestone");
      expect(listedTheme({ theme: "dark", darkTheme: bad === "slate" ? "limestone" : bad })).toBe("slate");
    }
  });

  it("saves a theme as its scheme and its id, and System alone", () => {
    expect(choiceSettings("marble")).toEqual({ theme: "light", lightTheme: "marble" });
    expect(choiceSettings("limestone")).toEqual({ theme: "light", lightTheme: "limestone" });
    expect(choiceSettings("graphite")).toEqual({ theme: "dark", darkTheme: "graphite" });
    expect(choiceSettings("slate")).toEqual({ theme: "dark", darkTheme: "slate" });
    expect(choiceSettings("system")).toEqual({ theme: "system" });
  });

  it("saves nothing for an entry it does not have", () => {
    expect(choiceSettings("sandstone")).toEqual({});
    expect(choiceSettings("light")).toEqual({});
  });

  it("shows again what was picked, for every theme", () => {
    for (const t of THEMES) expect(listedTheme({ theme: "system", ...choiceSettings(t.id) })).toBe(t.id);
  });
});

describe("the theme on screen", () => {
  const s = { lightTheme: "marble", darkTheme: "graphite" };

  it("follows the system with System, and not otherwise", () => {
    expect(themeInUse({ theme: "system", ...s }, false).id).toBe("marble");
    expect(themeInUse({ theme: "system", ...s }, true).id).toBe("graphite");
    expect(themeInUse({ theme: "light", ...s }, true).id).toBe("marble");
    expect(themeInUse({ theme: "dark", ...s }, false).id).toBe("graphite");
  });

  it("is the default when none or an unknown one is chosen", () => {
    expect(themeInUse({ theme: "system" }, false).id).toBe("limestone");
    expect(themeInUse({ theme: "system", darkTheme: "sandstone" }, true).id).toBe("slate");
  });
});
