// The Android Back button closes the overlay opened last (menu, dialog,
// sheet, quick switcher, drawer) before it leaves the app. Overlays register
// a close function for as long as they are open.

const open: (() => void)[] = [];

/** Let Back close an overlay; call the returned function once it has closed. */
export function closeOnBack(close: () => void): () => void {
  const entry = () => close();
  open.push(entry);
  return () => {
    const i = open.indexOf(entry);
    if (i >= 0) open.splice(i, 1);
  };
}

/** Close the overlay opened last. False when none is open. */
export function back(): boolean {
  const close = open.pop();
  close?.();
  return !!close;
}
