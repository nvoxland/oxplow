import { useState } from "react";
import { Check, Copy } from "lucide-react";

/**
 * The Copy on a markdown code block, the same in the viewer and the
 * editor: `read` gives the block's text from the button (its block is
 * the button's parent). It says "Copied" for a moment after a copy.
 */
export function CopyCodeButton({ read }: { read(button: HTMLButtonElement): string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      className="oxplow-md-code-copy"
      data-testid="markdown-code-copy"
      contentEditable={false}
      title={copied ? "Copied" : "Copy"}
      aria-label={copied ? "Copied" : "Copy code"}
      // Keep the editor's selection where it is.
      onMouseDown={(e) => e.preventDefault()}
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        const text = read(e.currentTarget);
        void navigator.clipboard
          .writeText(text)
          .then(() => {
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1200);
          })
          .catch(() => {});
      }}
    >
      {copied ? <Check size={12} /> : <Copy size={12} />}
    </button>
  );
}
