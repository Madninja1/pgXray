/**
 * Convert a bounded PTY diagnostic tail into readable *plain text*.
 * This is not a terminal emulator: interactive output must still go to xterm.js.
 * Terminal cursor movements/redraws cannot be reconstructed accurately here.
 */
export function ptyDiagnosticText(raw: string): string {
  // OSC (window title, links, etc.), terminated by BEL or ST.
  const withoutOsc = raw.replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, "");
  // CSI (SGR colors, bracketed paste, cursor movement, erased lines, etc.).
  const withoutCsi = withoutOsc.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "");
  // Remaining short ESC commands and incomplete escape tails from the ring buffer.
  const text = withoutCsi
    .replace(/\x1b(?:[()][0-~]|[ -/]*[@-~])?/g, "")
    .replace(/\r\n?/g, "\n");

  let result = "";
  for (const char of text) {
    if (char === "\b") {
      // Backspace edits only the current line, never the previous one.
      if (result.length && result[result.length - 1] !== "\n") result = result.slice(0, -1);
    } else if (char === "\n" || char === "\t" || char >= " ") {
      if (char !== "\x7f") result += char;
    }
  }
  return result;
}
