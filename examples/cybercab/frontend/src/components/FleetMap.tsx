import { useEffect, useRef } from "react";
import maplibregl, { type GeoJSONSource, type Map as MapLibre } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";

import type { Cab, Depot, FleetAlert, ServiceZone, TelemetrySample, Trip } from "../lib/client";
import { CabLayer, type CabMark } from "../lib/cabLayer";
import { CAB_STATUS, cabStatus } from "../lib/model";
import { circle, progressAlong, route, type LngLat } from "../lib/geo";

const STYLE = "https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json";
const LABEL_FONT = ["Montserrat Medium", "Open Sans Bold", "Noto Sans Regular"];
const AUSTIN: LngLat = [-97.728, 30.282];
/** Reported positions to keep per cab for its light trail. */
const TRAIL_LENGTH = 26;
/** How often trails, zones, hubs, alerts and the journey are redrawn, at most. */
const SLOW_MS = 250;

type Feature = GeoJSON.Feature<GeoJSON.Geometry, Record<string, unknown>>;
const collection = (features: Feature[]): GeoJSON.FeatureCollection => ({ type: "FeatureCollection", features });

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
 *
 * Live updates arrive hundreds of times a second at fleet scale, so they only mark what
 * changed. Each animation frame hands the cabs to a WebGL layer, which glides them on the
 * GPU, and the slower layers are redrawn a few times a second.
 */
export function FleetMap(props: FleetMapProps) {
  const container = useRef<HTMLDivElement>(null);
  const map = useRef<MapLibre>(undefined);
  const ready = useRef(false);
  const latest = useRef(props);
  latest.current = props;
  const changed = useRef({ cabs: true, slow: true });
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
    const cabs = new CabLayer(AUSTIN, props.wall ? 1.35 : 1);
    const callout = document.createElement("div");
    callout.className = "cab-callout";
    const label = new maplibregl.Marker({ element: callout, anchor: "top", offset: [0, 14] });
    const trails = new Map<string, LngLat[]>();

    instance.on("load", () => {
      retint(instance);
      for (const id of ["zones", "depots", "trails", "journey", "stops", "alerts"]) {
        instance.addSource(id, { type: "geojson", data: collection([]) });
      }
      addLayers(instance);
      instance.addLayer(cabs);
      instance.on("click", (event) => latest.current.onSelectCab(cabs.pick(event.point)));
      let hovering = 0;
      instance.on("mousemove", (event) => {
        cancelAnimationFrame(hovering);
        hovering = requestAnimationFrame(() => {
          instance.getCanvas().style.cursor = cabs.pick(event.point) ? "pointer" : "";
        });
      });
      ready.current = true;
    });

    // Each frame: hand the cabs over if they changed, keep the selected cab's label and
    // the camera with it, and now and then redraw the slower layers.
    let frame = 0;
    let lastSlow = -Infinity;
    let following: LngLat | undefined;
    const animate = (now: number) => {
      frame = requestAnimationFrame(animate);
      if (!ready.current) return;
      const current = latest.current;
      if (changed.current.cabs) {
        changed.current.cabs = false;
        cabs.setMarks(marks(current));
      }
      if (now - lastSlow >= SLOW_MS) {
        lastSlow = now;
        extendTrails(trails, current.cabs);
        drawTrails(instance, current.cabs, trails);
        if (changed.current.slow) {
          changed.current.slow = false;
          drawStatic(instance, current);
        }
      }
      const selected = current.cabs.find((cab) => cab.id === current.selectedCabId);
      const at = selected && cabs.positionOf(selected.id, now);
      if (selected && at) {
        callout.textContent = selected.callSign;
        label.setLngLat(at).addTo(instance);
        const target: LngLat = [selected.lng, selected.lat];
        if (current.follow && !flying.current && (following?.[0] !== target[0] || following?.[1] !== target[1])) {
          following = target;
          instance.easeTo({ center: target, duration: 1000, easing: (t) => t });
        }
      } else {
        label.remove();
        following = undefined;
      }
      // Alert rings breathe.
      if (instance.getLayer("alert-rings")) {
        const phase = (now % 1800) / 1800;
        instance.setPaintProperty("alert-rings", "circle-radius", 10 + phase * 16);
        instance.setPaintProperty("alert-rings", "circle-stroke-opacity", 0.9 * (1 - phase));
      }
    };
    frame = requestAnimationFrame(animate);
    return () => {
      cancelAnimationFrame(frame);
      instance.remove();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Updates only mark what changed; the next frame draws it.
  useEffect(() => {
    changed.current.cabs = true;
  }, [props.cabs, props.selectedCabId]);
  useEffect(() => {
    changed.current.slow = true;
  }, [props.zones, props.alerts, props.trips, props.trail, props.selectedCabId, props.depots, props.cabs]);

  // Fly to a newly selected cab; the frame loop follows it from there.
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

  // MapLibre makes its container `position: relative`, so it lives inside the stage-filling box.
  return (
    <div className="fleet-map">
      <div ref={container} className="fleet-map__canvas" />
    </div>
  );
}

/** What the cab layer draws for each cab. */
function marks(props: FleetMapProps): CabMark[] {
  return props.cabs.map((cab) => {
    const status = cabStatus(cab);
    const selected = cab.id === props.selectedCabId;
    return {
      id: cab.id,
      at: [cab.lng, cab.lat],
      heading: cab.headingDeg,
      color: cab.halted ? "#f0a84a" : CAB_STATUS[status].color,
      halo: selected || status === "on_trip",
      selected,
      dim: !!props.selectedCabId && !selected,
    };
  });
}

/** Adds each cab's newest reported position to its trail. */
function extendTrails(trails: Map<string, LngLat[]>, cabs: Cab[]) {
  for (const cab of cabs) {
    const trail = trails.get(cab.id) ?? [];
    const last = trail[trail.length - 1];
    if (!last || last[0] !== cab.lng || last[1] !== cab.lat) {
      trail.push([cab.lng, cab.lat]);
      if (trail.length > TRAIL_LENGTH) trail.shift();
      trails.set(cab.id, trail);
    }
  }
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

function addLayers(map: MapLibre) {
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
}

function drawTrails(map: MapLibre, cabs: Cab[], trails: Map<string, LngLat[]>) {
  const features: Feature[] = [];
  for (const cab of cabs) {
    const trail = trails.get(cab.id);
    if (!trail || trail.length < 2 || cab.speedKph === 0) continue;
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
    const event = !!zone.eventName;
    const label = `${zone.name}${zone.surge > 1 ? `  ${zone.surge.toFixed(1)}×` : ""}${event ? ` · ${zone.eventName}` : ""}`;
    const properties = { surge: zone.surge, event, label };
    zones.push({ type: "Feature", geometry: { type: "Polygon", coordinates: [circle([zone.lng, zone.lat], zone.radiusM)] }, properties });
    zones.push({ type: "Feature", geometry: { type: "Point", coordinates: [zone.lng, zone.lat + zone.radiusM / 111_000] }, properties });
  }
  (map.getSource("zones") as GeoJSONSource).setData(collection(zones));

  (map.getSource("depots") as GeoJSONSource).setData(
    collection(
      props.depots.map((depot) => {
        const charging = props.cabs.filter((c) => c.depotId === depot.id && c.status === "charging").length;
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
  const trip = cab?.tripId ? props.trips.get(cab.tripId) : undefined;
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
    const approach = route(trip.approachPolyline);
    const ride = route(trip.ridePolyline);
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
      { type: "Feature", geometry: { type: "Point", coordinates: [trip.pickupLng, trip.pickupLat] }, properties: { color: "#45c8e8" } },
      { type: "Feature", geometry: { type: "Point", coordinates: [trip.dropoffLng, trip.dropoffLat] }, properties: { color: "#d9b97a" } },
    );
  }
  (map.getSource("journey") as GeoJSONSource).setData(collection(journey));
  (map.getSource("stops") as GeoJSONSource).setData(collection(stops));
}
