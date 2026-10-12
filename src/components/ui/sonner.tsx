import type { CSSProperties } from "react";
import { Toaster as Sonner, type ToasterProps } from "sonner";

/** Toast host, themed with Lichtwart tokens. */
function Toaster(props: ToasterProps) {
  return (
    <Sonner
      className="toaster group"
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border)",
          "--border-radius": "var(--lw-r-md)",
          fontFamily: "var(--lw-font-sans)",
        } as CSSProperties
      }
      {...props}
    />
  );
}

export { Toaster };
