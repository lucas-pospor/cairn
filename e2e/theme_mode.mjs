// Settings, then Appearance, then Theme, for tests that only need light, dark
// or System. The list holds System and each theme by name, so "light" picks the
// light theme the page uses now (Limestone unless another one is chosen) and
// "dark" the dark one, as the Light and Dark entries of 1.2.0 did. Picking a
// theme saves "theme" and that theme's lightTheme or darkTheme.

/**
 * Page JS (a block, so it can be put before other statements) that picks
 * `mode` ("system", "light" or "dark") in the open Appearance settings.
 */
export const pickThemeMode = (mode) => `{
  const s = document.querySelector('[data-testid=theme-select]'), m = ${JSON.stringify(mode)};
  s.value = m === 'system' ? 'system' : document.documentElement.dataset[m === 'dark' ? 'darkTheme' : 'lightTheme'];
  if (s.selectedIndex < 0) throw new Error('no theme for ' + m + ' in the Theme list');
  s.dispatchEvent(new Event('change', { bubbles: true }));
}`;
