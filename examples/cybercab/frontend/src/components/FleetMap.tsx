import { useEffect, useRef } from "react";
import maplibregl, { type GeoJSONSource, type Map as MapLibre } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";

import type { Cab, Depot, FleetAlert, ServiceZone, TelemetrySample, Trip } from "../lib/client";
import { CAB_STATUS, cabStatus } from "../lib/model";
import { circle, progressAlong, route, type LngLat } from "../lib/geo";

const STYLE = "https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json";
const LABEL_FONT = ["Montserrat Medium", "Open Sans Bold", "Noto Sans Regular"];
const AUSTIN: LngLat = [-97.728, 30.282];
/** Reported positions to keep per cab for its light trail. */
const TRAIL_LENGTH = 26;

type Feature = GeoJSON.Feature<GeoJSON.Geometry, Record<string, unknown>>;
const collection = (features: Feature[]): GeoJSON.FeatureCollection => ({ type: "FeatureCollection", features });

/** A chevron pointing north, drawn once and tinted per cab (an SDF icon). */
function chevron(): ImageData {
  const size = 64;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const g = canvas.getContext("2d")!;
  g.fillStyle = "#fff";
  g.beginPath();
  g.moveTo(32, 6);
  g.lineTo(52, 54);
  g.lineTo(32, 43);
  g.lineTo(12, 54);
  g.closePath();
  g.fill();
  return g.getImageData(0, 0, size, size);
}

interface Glide {
  from: LngLat;
  to: LngLat;
  started: number;
}

export interface FleetMapProps {
  cabs: Cab[];
  trips: Map<string, Trip>;
  zones: ServiceZone[];
  depots: Depot[];
  alerts: FleetAlert[];
  trail: TelemetrySample[];
  selectedCabId?: string;
  /** Keep the camera on the selected cab, as wall mode's spotlight does. */
  follow?: boolean;
  wall?: boolean;
  onSelectCab: (id: string | undefined) => void;
}

/**
 * The city at night with the fleet on it. Cabs glide between their once-a-second
 * reports, leave light trails, and the selected cab's journey is drawn in full: the
 * trail it left, the approach to the pickup, and the ride to the drop-off.
 */
export function FleetMap(props: FleetMapProps) {
  const container = useRef<HTMLDivElement>(null);
  const map = useRef<MapLibre>(undefined);
  const ready = useRef(false);
  const latest = useRef(props);
  latest.current = props;
  const glides = useRef(new Map<string, Glide>());
  const trails = useRef(new Map<string, LngLat[]>());
  const flying = useRef(false);

  // The map, once.
  useEffect(() => {
    const instance = new maplibregl.Map({
      container: container.current!,
      style: STYLE,
      center: AUSTIN,
      zoom: props.wall ? 11.4 : 11.9,
      pitch: props.wall ? 38 : 0,
      attributionControl: false,
      fadeDuration: 0,
    });
    // Centre the city in what the docked instruments leave visible.
    instance.setPadding({ top: 0, bottom: 0, left: 0, right: props.wall ? 500 : 420 });
    map.current = instance;
    instance.addControl(new maplibregl.AttributionControl({ compact: true }), "bottom-left");
    instance.on("load", () => {
      retint(instance);
      instance.addImage("cab", chevron(), { sdf: true, pixelRatio: 2 });
      for (const id of ["zones", "depots", "trails", "journey", "stops", "alerts", "cabs"]) {
        instance.addSource(id, { type: "geojson", data: collection([]) });
      }
      addLayers(instance, !!props.wall);
      instance.on("click", "cab-icons", (event) => {
        const id = event.features?.[0]?.properties?.id as string | undefined;
        latest.current.onSelectCab(id);
      });
      instance.on("click", (event) => {
        const hits = instance.queryRenderedFeatures(event.point, { layers: ["cab-icons"] });
        if (hits.length === 0) latest.current.onSelectCab(undefined);
      });
      instance.on("mouseenter", "cab-icons", () => (instance.getCanvas().style.cursor = "pointer"));
      instance.on("mouseleave", "cab-icons", () => (instance.getCanvas().style.cursor = ""));
      ready.current = true;
      drawStatic(instance, latest.current);
    });
    // Cabs glide between reports; alerts breathe.
    let frame = 0;
    const animate = (now: number) => {
      if (ready.current) drawCabs(instance, latest.current, glides.current, now);
      frame = requestAnimationFrame(animate);
    };
    frame = requestAnimationFrame(animate);
    return () => {
      cancelAnimationFrame(frame);
      instance.remove();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // New reports: start a glide from wherever each cab is drawn, and extend its trail.
  useEffect(() => {
    const now = performance.now();
    for (const cab of props.cabs) {
      const to: LngLat = [cab.lng, cab.lat];
      const glide = glides.current.get(cab.id);
      if (!glide) {
        glides.current.set(cab.id, { from: to, to, started: now });
      } else if (glide.to[0] !== to[0] || glide.to[1] !== to[1]) {
        glides.current.set(cab.id, { from: position(glide, now), to, started: now });
        const trail = trails.current.get(cab.id) ?? [];
        trail.push(to);
        if (trail.length > TRAIL_LENGTH) trail.shift();
        trails.current.set(cab.id, trail);
      }
    }
    if (ready.current && map.current) drawTrails(map.current, props.cabs, trails.current);
  }, [props.cabs]);

  useEffect(() => {
    if (ready.current && map.current) drawStatic(map.current, props);
  }, [props.zones, props.alerts, props.trips, props.trail, props.selectedCabId, props.depots, props.cabs]);

  // Fly to a newly selected cab.
  useEffect(() => {
    const cab = props.cabs.find((c) => c.id === props.selectedCabId);
    if (!cab || !map.current) return;
    flying.current = true;
    map.current.once("moveend", () => (flying.current = false));
    map.current.flyTo({
      center: [cab.lng, cab.lat],
      zoom: Math.max(map.current.getZoom(), props.wall ? 13.2 : 13),
      speed: 0.9,
      curve: 1.4,
      essential: true,
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.selectedCabId]);

  // Follow it while it drives, once the flight to it has landed.
  useEffect(() => {
    if (!props.follow || !map.current) return;
    const cab = props.cabs.find((c) => c.id === props.selectedCabId);
    if (cab && !flying.current) {
      map.current.easeTo({ center: [cab.lng, cab.lat], duration: 1000, easing: (t) => t });
    }
  }, [props.cabs, props.follow, props.selectedCabId]);

  // MapLibre makes its container `position: relative`, so it lives inside the stage-filling box.
  return (
    <div className="fleet-map">
      <div ref={container} className="fleet-map__canvas" />
    </div>
  );
}

function position(glide: Glide, now: number): LngLat {
  const t = Math.min(1, (now - glide.started) / 1000);
  return [glide.from[0] + (glide.to[0] - glide.from[0]) * t, glide.from[1] + (glide.to[1] - glide.from[1]) * t];
}

/** Takes the basemap down into the room's asphalt. */
function retint(map: MapLibre) {
  const set = (layer: string, property: string, value: unknown) => {
    if (map.getLayer(layer)) map.setPaintProperty(layer, property, value);
  };
  set("background", "background-color", "#06090e");
  set("water", "fill-color", "#0a1522");
  set("landcover", "fill-color", "#080c12");
  set("landuse", "fill-color", "#080c12");
  set("landuse_residential", "fill-color", "#080b11");
  set("building", "fill-color", "#0c1119");
  set("building-top", "fill-color", "#0e141d");
  for (const layer of map.getStyle().layers) {
    if (layer.type === "line" && /^(road|bridge|tunnel)_.*_fill/.test(layer.id)) {
      const major = /(mot|trunk|pri)/.test(layer.id);
      map.setPaintProperty(layer.id, "line-color", major ? "#1f2a38" : "#141b25");
    }
    if (layer.type === "symbol" && /^place_/.test(layer.id)) {
      map.setPaintProperty(layer.id, "text-color", "#4a5466");
      map.setPaintProperty(layer.id, "text-halo-color", "#06090e");
    }
  }
}

function addLayers(map: MapLibre, wall: boolean) {
  const iconScale = wall ? 1.35 : 1;
  map.addLayer({
    id: "zone-fill",
    type: "fill",
    source: "zones",
    paint: {
      "fill-color": ["case", ["get", "event"], "#d9b97a", "#8b9db6"],
      "fill-opacity": ["interpolate", ["linear"], ["get", "surge"], 1, 0.025, 2.5, 0.14],
    },
  });
  map.addLayer({
    id: "zone-edge",
    type: "line",
    source: "zones",
    paint: {
      "line-color": ["case", ["get", "event"], "#d9b97a", "rgba(148,170,200,0.22)"],
      "line-width": ["case", ["get", "event"], 1.4, 0.8],
      "line-dasharray": [2, 3],
    },
  });
  map.addLayer({
    id: "zone-label",
    type: "symbol",
    source: "zones",
    filter: ["==", ["geometry-type"], "Point"],
    layout: {
      "text-field": ["get", "label"],
      "text-font": LABEL_FONT,
      "text-size": 11,
      "text-letter-spacing": 0.12,
      "text-transform": "uppercase",
    },
    paint: {
      "text-color": ["case", ["get", "event"], "#d9b97a", "rgba(170,181,196,0.55)"],
      "text-halo-color": "#06090e",
      "text-halo-width": 1.5,
    },
  });
  map.addLayer({
    id: "depots",
    type: "circle",
    source: "depots",
    paint: {
      "circle-radius": 5,
      "circle-color": "#06090e",
      "circle-stroke-width": 2,
      "circle-stroke-color": "#5fd391",
    },
  });
  map.addLayer({
    id: "depot-label",
    type: "symbol",
    source: "depots",
    layout: {
      "text-field": ["get", "label"],
      "text-font": LABEL_FONT,
      "text-size": 10,
      "text-offset": [0, 1.2],
      "text-anchor": "top",
      "text-letter-spacing": 0.1,
    },
    paint: { "text-color": "rgba(95,211,145,0.75)", "text-halo-color": "#06090e", "text-halo-width": 1.5 },
  });
  map.addLayer({
    id: "trails",
    type: "line",
    source: "trails",
    layout: { "line-cap": "round" },
    paint: {
      "line-color": ["get", "color"],
      "line-width": ["interpolate", ["linear"], ["zoom"], 11, 2.2, 15, 4],
      "line-opacity": ["get", "opacity"],
      "line-blur": 0.8,
    },
  });
  map.addLayer({
    id: "journey",
    type: "line",
    source: "journey",
    layout: { "line-cap": "round", "line-join": "round" },
    paint: {
      "line-color": ["get", "color"],
      "line-width": ["get", "width"],
      "line-opacity": ["get", "opacity"],
      "line-dasharray": [1, 0],
    },
  });
  map.addLayer({
    id: "stops",
    type: "circle",
    source: "stops",
    paint: {
      "circle-radius": 7,
      "circle-color": "#06090e",
      "circle-stroke-width": 2.5,
      "circle-stroke-color": ["get", "color"],
    },
  });
  map.addLayer({
    id: "alert-rings",
    type: "circle",
    source: "alerts",
    paint: {
      "circle-radius": 14,
      "circle-color": "transparent",
      "circle-stroke-width": 1.5,
      "circle-stroke-color": ["get", "color"],
      "circle-stroke-opacity": 0.8,
    },
  });
  map.addLayer({
    id: "cab-halo",
    type: "circle",
    source: "cabs",
    filter: ["any", ["get", "selected"], ["==", ["get", "status"], "on_trip"]],
    paint: {
      "circle-radius": ["case", ["get", "selected"], 20, 11],
      "circle-color": ["get", "color"],
      "circle-opacity": ["case", ["get", "selected"], 0.28, 0.14],
      "circle-blur": 0.9,
    },
  });
  map.addLayer({
    id: "cab-icons",
    type: "symbol",
    source: "cabs",
    layout: {
      "icon-image": "cab",
      "icon-size": ["interpolate", ["linear"], ["zoom"], 10, 0.42 * iconScale, 13, 0.62 * iconScale, 16, 0.85 * iconScale],
      "icon-rotate": ["get", "heading"],
      "icon-rotation-alignment": "map",
      "icon-pitch-alignment": "map",
      "icon-allow-overlap": true,
      "icon-ignore-placement": true,
      "symbol-sort-key": ["get", "rank"],
    },
    paint: {
      "icon-color": ["get", "color"],
      "icon-opacity": ["case", ["get", "dim"], 0.45, 1],
      "icon-halo-color": "#06090e",
      "icon-halo-width": 1,
    },
  });
  map.addLayer({
    id: "cab-label",
    type: "symbol",
    source: "cabs",
    filter: ["get", "selected"],
    layout: {
      "text-field": ["get", "call_sign"],
      "text-font": LABEL_FONT,
      "text-size": 12,
      "text-offset": [0, 1.6],
      "text-anchor": "top",
      "text-letter-spacing": 0.08,
    },
    paint: { "text-color": "#e9eef5", "text-halo-color": "#06090e", "text-halo-width": 2 },
  });
}

function drawCabs(map: MapLibre, props: FleetMapProps, glides: Map<string, Glide>, now: number) {
  const source = map.getSource("cabs") as GeoJSONSource | undefined;
  if (!source) return;
  const features: Feature[] = props.cabs.map((cab) => {
    const status = cabStatus(cab);
    const glide = glides.get(cab.id);
    const at = glide ? position(glide, now) : ([cab.lng, cab.lat] as LngLat);
    const selected = cab.id === props.selectedCabId;
    return {
      type: "Feature",
      geometry: { type: "Point", coordinates: at },
      properties: {
        id: cab.id,
        call_sign: cab.call_sign,
        status,
        color: cab.halted ? "#f0a84a" : CAB_STATUS[status].color,
        heading: cab.heading_deg,
        selected,
        dim: !!props.selectedCabId && !selected,
        rank: selected ? 10 : status === "on_trip" ? 5 : 1,
      },
    };
  });
  source.setData(collection(features));
  // Alert rings breathe.
  if (map.getLayer("alert-rings")) {
    const phase = (now % 1800) / 1800;
    map.setPaintProperty("alert-rings", "circle-radius", 10 + phase * 16);
    map.setPaintProperty("alert-rings", "circle-stroke-opacity", 0.9 * (1 - phase));
  }
}

function drawTrails(map: MapLibre, cabs: Cab[], trails: Map<string, LngLat[]>) {
  const features: Feature[] = [];
  for (const cab of cabs) {
    const trail = trails.get(cab.id);
    if (!trail || trail.length < 2 || cab.speed_kph === 0) continue;
    const color = CAB_STATUS[cabStatus(cab)].color;
    for (let i = 1; i < trail.length; i++) {
      features.push({
        type: "Feature",
        geometry: { type: "LineString", coordinates: [trail[i - 1], trail[i]] },
        properties: { color, opacity: 0.8 * (i / trail.length) ** 1.3 },
      });
    }
  }
  (map.getSource("trails") as GeoJSONSource).setData(collection(features));
}

function drawStatic(map: MapLibre, props: FleetMapProps) {
  const zones: Feature[] = [];
  for (const zone of props.zones) {
    const event = !!zone.event_name;
    const label = `${zone.name}${zone.surge > 1 ? `  ${zone.surge.toFixed(1)}×` : ""}${event ? ` · ${zone.event_name}` : ""}`;
    const properties = { surge: zone.surge, event, label };
    zones.push({ type: "Feature", geometry: { type: "Polygon", coordinates: [circle([zone.lng, zone.lat], zone.radius_m)] }, properties });
    zones.push({ type: "Feature", geometry: { type: "Point", coordinates: [zone.lng, zone.lat + zone.radius_m / 111_000] }, properties });
  }
  (map.getSource("zones") as GeoJSONSource).setData(collection(zones));

  (map.getSource("depots") as GeoJSONSource).setData(
    collection(
      props.depots.map((depot) => {
        const charging = props.cabs.filter((c) => c.depot_id === depot.id && c.status === "charging").length;
        const label = `${depot.code}${charging ? ` · ${charging} charging` : ""}`;
        return { type: "Feature", geometry: { type: "Point", coordinates: [depot.lng, depot.lat] }, properties: { label } };
      }),
    ),
  );

  const alertColor = { CRITICAL: "#f25b5b", WARNING: "#f0a84a", INFO: "#45c8e8" } as Record<string, string>;
  (map.getSource("alerts") as GeoJSONSource).setData(
    collection(
      props.alerts
        .filter((alert) => alert.status === "open" && alert.severity !== "INFO")
        .map((alert) => ({
          type: "Feature",
          geometry: { type: "Point", coordinates: [alert.lng, alert.lat] },
          properties: { color: alertColor[alert.severity] ?? "#f0a84a" },
        })),
    ),
  );

  // The selected cab's journey.
  const journey: Feature[] = [];
  const stops: Feature[] = [];
  const cab = props.cabs.find((c) => c.id === props.selectedCabId);
  const trip = cab?.trip_id ? props.trips.get(cab.trip_id) : undefined;
  const line = (coordinates: LngLat[], color: string, width: number, opacity: number) => {
    if (coordinates.length > 1) {
      journey.push({ type: "Feature", geometry: { type: "LineString", coordinates }, properties: { color, width, opacity } });
    }
  };
  if (props.trail.length > 1) {
    line(
      [...props.trail].reverse().map((s) => [s.lng, s.lat] as LngLat),
      "#aab5c4",
      1.5,
      0.45,
    );
  }
  if (cab && trip) {
    const at: LngLat = [cab.lng, cab.lat];
    const approach = route(trip.approach_polyline);
    const ride = route(trip.ride_polyline);
    if (trip.status === "assigned" && approach.length > 1) {
      const split = progressAlong(approach, at);
      line(split.ahead, "#45c8e8", 3.5, 0.95);
      line(ride, "#d9b97a", 2.5, 0.4);
    } else if (trip.status === "riding" && ride.length > 1) {
      const split = progressAlong(ride, at);
      line(split.behind, "#d9b97a", 2.5, 0.35);
      line(split.ahead, "#d9b97a", 4, 0.95);
    } else if (ride.length > 1) {
      line(ride, "#d9b97a", 3.5, 0.85);
    }
    stops.push(
      { type: "Feature", geometry: { type: "Point", coordinates: [trip.pickup_lng, trip.pickup_lat] }, properties: { color: "#45c8e8" } },
      { type: "Feature", geometry: { type: "Point", coordinates: [trip.dropoff_lng, trip.dropoff_lat] }, properties: { color: "#d9b97a" } },
    );
  }
  (map.getSource("journey") as GeoJSONSource).setData(collection(journey));
  (map.getSource("stops") as GeoJSONSource).setData(collection(stops));
}
