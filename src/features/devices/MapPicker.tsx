import L from "leaflet";
import "leaflet/dist/leaflet.css";
import { useEffect, useRef } from "react";

import { cn } from "@/lib/utils";

export interface LatLng {
  lat: number;
  lng: number;
}

const DEFAULT_CENTER: LatLng = { lat: 51.1657, lng: 10.4515 }; // Germany
const OSM_TILES = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || "currentColor";
}

/**
 * OpenStreetMap picker (PRD §19: Leaflet + OSM tiles). Click sets the position.
 * The marker is a vector circle so no marker image assets are needed.
 */
export function MapPicker({
  value,
  onChange,
  label,
  className,
}: {
  value: LatLng | null;
  onChange: (pos: LatLng) => void;
  /** Accessible name of the map region. */
  label: string;
  className?: string;
}) {
  const el = useRef<HTMLDivElement>(null);
  const map = useRef<L.Map | null>(null);
  const marker = useRef<L.CircleMarker | null>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const initial = useRef(value);

  useEffect(() => {
    if (!el.current) return;
    const start = initial.current;
    const m = L.map(el.current, { zoomControl: true, attributionControl: true }).setView(
      start ?? DEFAULT_CENTER,
      start ? 13 : 5,
    );
    L.tileLayer(OSM_TILES, {
      maxZoom: 19,
      attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a>',
    }).addTo(m);
    m.on("click", (e: L.LeafletMouseEvent) =>
      onChangeRef.current({ lat: e.latlng.lat, lng: e.latlng.lng }),
    );
    map.current = m;
    // The dialog animates in: re-measure once it has its final size.
    const timer = setTimeout(() => m.invalidateSize(), 250);
    return () => {
      clearTimeout(timer);
      m.remove();
      map.current = null;
      marker.current = null;
    };
  }, []);

  useEffect(() => {
    const m = map.current;
    if (!m) return;
    if (!value) {
      marker.current?.remove();
      marker.current = null;
      return;
    }
    if (marker.current) marker.current.setLatLng(value);
    else {
      marker.current = L.circleMarker(value, {
        radius: 9,
        color: cssVar("--lw-night-blue"),
        weight: 2,
        fillColor: cssVar("--lw-white"),
        fillOpacity: 1,
      }).addTo(m);
    }
    if (!m.getBounds().contains(value)) m.panTo(value);
  }, [value]);

  return (
    <div
      ref={el}
      role="application"
      aria-label={label}
      className={cn("relative z-0 h-56 w-full overflow-hidden rounded-md border", className)}
    />
  );
}
