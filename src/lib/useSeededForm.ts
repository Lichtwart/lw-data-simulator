import { useEffect, useRef, useState } from "react";

/**
 * Local form state seeded from server data. A new seed (after a reload) replaces the form only
 * while the form is clean - so saving one card never throws away unsaved edits in another.
 *
 * - `dirty`: the form differs from the current seed.
 * - `markSaved()`: call right after a successful save, so the incoming new seed replaces the form.
 * - `discard()`: reset the form to the current seed.
 * `serialize` produces the comparison string (defaults to JSON; pass one that ignores client-only fields).
 */
export function useSeededForm<T>(seed: T, serialize: (v: T) => string = JSON.stringify) {
  const [value, setValue] = useState<T>(seed);
  const seedKey = serialize(seed);
  const lastSeedKey = useRef(seedKey);
  const valueRef = useRef(value);
  valueRef.current = value;
  const seedRef = useRef(seed);
  seedRef.current = seed;
  const serializeRef = useRef(serialize);
  serializeRef.current = serialize;

  useEffect(() => {
    if (seedKey === lastSeedKey.current) return;
    const clean = serializeRef.current(valueRef.current) === lastSeedKey.current;
    lastSeedKey.current = seedKey;
    if (clean) setValue(seedRef.current);
  }, [seedKey]);

  return {
    value,
    setValue,
    dirty: serialize(value) !== seedKey,
    discard: () => setValue(seedRef.current),
    markSaved: () => {
      lastSeedKey.current = serializeRef.current(valueRef.current);
    },
  };
}
