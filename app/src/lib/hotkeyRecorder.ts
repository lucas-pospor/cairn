// Set while the settings screen is recording a new hotkey, so the global
// key handler does not run commands in the meantime.
let recording = false;
export const recordingHotkey = () => recording;
export const setRecordingHotkey = (v: boolean) => {
  recording = v;
};
