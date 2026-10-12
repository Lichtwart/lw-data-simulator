import { useEffect, useState } from "react";

/**
 * Current time, re-read every `intervalMs` (for countdowns). Ticks are aligned to the interval
 * boundary (e.g. full seconds) so a countdown never skips or repeats a number.
 */
export function useNow(intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    let id: ReturnType<typeof setTimeout>;
    const schedule = () => {
      id = setTimeout(
        () => {
          setNow(Date.now());
          schedule();
        },
        intervalMs - (Date.now() % intervalMs) + 5,
      );
    };
    setNow(Date.now());
    schedule();
    return () => clearTimeout(id);
  }, [intervalMs]);
  return now;
}
