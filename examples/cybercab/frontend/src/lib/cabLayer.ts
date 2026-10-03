import maplibregl, { type CustomLayerInterface, type CustomRenderMethodInput, type Map as MapLibre } from "maplibre-gl";

import type { LngLat } from "./geo";

/** How long a cab takes to glide from one reported position to the next. */
const GLIDE_MS = 1000;

/** What the layer draws for one cab. */
export interface CabMark {
  id: string;
  at: LngLat;
  /** Degrees clockwise from north. */
  heading: number;
  /** `#rrggbb`. */
  color: string;
  /** Drawn with a soft halo: carrying a rider, or selected. */
  halo: boolean;
  selected: boolean;
  /** Faded back, while another cab is selected. */
  dim: boolean;
}

interface Glide {
  from: LngLat;
  to: LngLat;
  started: number;
}

// Per instance: from (2), to (2), start (1), heading (1), rgb (3), size (1), alpha (1), halo (1).
const STRIDE = 12;

const VERTEX = `#version 300 es
precision highp float;
uniform mat4 u_matrix;
uniform vec2 u_viewport;
uniform float u_now;
uniform float u_bearing;
uniform float u_scale;
uniform float u_pass;
layout(location = 0) in vec2 a_corner;
layout(location = 1) in vec4 a_path;
layout(location = 2) in vec2 a_timing;
layout(location = 3) in vec3 a_color;
layout(location = 4) in vec3 a_style;
out vec2 v_local;
out vec3 v_color;
out float v_alpha;
void main() {
  float t = clamp((u_now - a_timing.x) / ${GLIDE_MS.toFixed(1)}, 0.0, 1.0);
  vec2 at = mix(a_path.xy, a_path.zw, t);
  vec4 clip = u_matrix * vec4(at, 0.0, 1.0);
  float halo = a_style.z;
  // The halo pass draws only cabs with a halo, a little wider than the chevron.
  float size = a_style.x * u_scale * (u_pass > 0.5 ? 1.6 : 1.0) * (u_pass > 0.5 && halo < 0.5 ? 0.0 : 1.0);
  float angle = radians(a_timing.y - u_bearing);
  // Clockwise on screen, where y runs down.
  mat2 rotate = mat2(cos(angle), sin(angle), -sin(angle), cos(angle));
  vec2 offset = (u_pass > 0.5 ? a_corner : rotate * a_corner) * size;
  clip.xy += offset * 2.0 / u_viewport * clip.w * vec2(1.0, -1.0);
  gl_Position = clip;
  v_local = a_corner;
  v_color = a_color;
  v_alpha = a_style.y;
}`;

const FRAGMENT = `#version 300 es
precision highp float;
uniform float u_pass;
in vec2 v_local;
in vec3 v_color;
in float v_alpha;
out vec4 color;
float edge(vec2 p, vec2 a, vec2 b) {
  return (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x);
}
void main() {
  vec2 p = v_local;
  if (u_pass > 0.5) {
    // A soft glow.
    float a = (1.0 - smoothstep(0.0, 1.0, length(p))) * 0.16 * v_alpha;
    color = vec4(v_color * a, a);
    return;
  }
  // A chevron pointing up the screen: tip, right wing, notch, left wing.
  vec2 tip = vec2(0.0, -0.85);
  vec2 right = vec2(0.65, 0.75);
  vec2 notch = vec2(0.0, 0.4);
  vec2 left = vec2(-0.65, 0.75);
  bool inside = (edge(p, tip, right) <= 0.0 && edge(p, right, notch) <= 0.0 && edge(p, notch, tip) <= 0.0)
    || (edge(p, tip, notch) <= 0.0 && edge(p, notch, left) <= 0.0 && edge(p, left, tip) <= 0.0);
  if (!inside) discard;
  color = vec4(v_color * v_alpha, v_alpha);
}`;

const colors = new Map<string, [number, number, number]>();

function hexColor(hex: string): [number, number, number] {
  let rgb = colors.get(hex);
  if (!rgb) {
    const value = Number.parseInt(hex.replace("#", ""), 16);
    rgb = [((value >> 16) & 255) / 255, ((value >> 8) & 255) / 255, (value & 255) / 255];
    colors.set(hex, rgb);
  }
  return rgb;
}

/** Web Mercator x and y, from 0 to 1, as `MercatorCoordinate.fromLngLat` gives them. */
function mercatorX(lng: number): number {
  return (180 + lng) / 360;
}

function mercatorY(lat: number): number {
  return (180 - (180 / Math.PI) * Math.log(Math.tan(Math.PI / 4 + (lat * Math.PI) / 360))) / 360;
}

/**
 * The fleet as a WebGL layer. Each cab's reported positions go to the GPU once, when they
 * change, and the shader glides it between them every frame: no per-frame GeoJSON, tiling
 * or symbol placement, however many cabs there are.
 */
export class CabLayer implements CustomLayerInterface {
  readonly id = "cabs";
  readonly type = "custom" as const;
  readonly renderingMode = "2d" as const;

  private map?: MapLibre;
  private gl?: WebGL2RenderingContext;
  private program?: WebGLProgram;
  private vao?: WebGLVertexArrayObject;
  private instances?: WebGLBuffer;
  private count = 0;
  private marks: CabMark[] = [];
  private dirty = false;
  private readonly glides = new Map<string, Glide>();
  private data = new Float32Array(0);
  /** Positions are uploaded relative to this, so they stay precise at street level. */
  private readonly origin: maplibregl.MercatorCoordinate;
  private uniforms: Record<string, WebGLUniformLocation | null> = {};
  /** Times go to the GPU from here, so they stay precise in 32-bit floats. */
  private readonly epoch = performance.now();

  constructor(
    center: LngLat,
    private readonly scale: number,
  ) {
    this.origin = maplibregl.MercatorCoordinate.fromLngLat(center);
  }

  /** The fleet as it is now; uploaded with the next frame. */
  setMarks(marks: CabMark[]): void {
    this.marks = marks;
    this.dirty = true;
    this.map?.triggerRepaint();
  }

  /** Where `id` is drawn at `now`. */
  positionOf(id: string, now = performance.now()): LngLat | undefined {
    const glide = this.glides.get(id);
    return glide && glideAt(glide, now);
  }

  /** The cab drawn nearest `point` on screen, within `radius` pixels. */
  pick(point: { x: number; y: number }, radius = 16): string | undefined {
    if (!this.map) return undefined;
    const now = performance.now();
    let best: string | undefined;
    let bestDistance = radius * radius;
    for (const [id, glide] of this.glides) {
      const at = this.map.project(glideAt(glide, now));
      const distance = (at.x - point.x) ** 2 + (at.y - point.y) ** 2;
      if (distance < bestDistance) {
        bestDistance = distance;
        best = id;
      }
    }
    return best;
  }

  onAdd(map: MapLibre, gl: WebGLRenderingContext | WebGL2RenderingContext): void {
    this.map = map;
    const gl2 = gl as WebGL2RenderingContext;
    this.gl = gl2;
    const compile = (type: number, source: string) => {
      const shader = gl2.createShader(type)!;
      gl2.shaderSource(shader, source);
      gl2.compileShader(shader);
      if (!gl2.getShaderParameter(shader, gl2.COMPILE_STATUS)) throw new Error(gl2.getShaderInfoLog(shader) ?? "shader");
      return shader;
    };
    const program = gl2.createProgram()!;
    gl2.attachShader(program, compile(gl2.VERTEX_SHADER, VERTEX));
    gl2.attachShader(program, compile(gl2.FRAGMENT_SHADER, FRAGMENT));
    gl2.linkProgram(program);
    if (!gl2.getProgramParameter(program, gl2.LINK_STATUS)) throw new Error(gl2.getProgramInfoLog(program) ?? "program");
    this.program = program;
    for (const name of ["u_matrix", "u_viewport", "u_now", "u_bearing", "u_scale", "u_pass"]) {
      this.uniforms[name] = gl2.getUniformLocation(program, name);
    }

    this.vao = gl2.createVertexArray()!;
    gl2.bindVertexArray(this.vao);
    const corners = gl2.createBuffer();
    gl2.bindBuffer(gl2.ARRAY_BUFFER, corners);
    gl2.bufferData(gl2.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl2.STATIC_DRAW);
    gl2.enableVertexAttribArray(0);
    gl2.vertexAttribPointer(0, 2, gl2.FLOAT, false, 0, 0);

    this.instances = gl2.createBuffer()!;
    gl2.bindBuffer(gl2.ARRAY_BUFFER, this.instances);
    const bytes = STRIDE * 4;
    const attribute = (location: number, size: number, offset: number) => {
      gl2.enableVertexAttribArray(location);
      gl2.vertexAttribPointer(location, size, gl2.FLOAT, false, bytes, offset * 4);
      gl2.vertexAttribDivisor(location, 1);
    };
    attribute(1, 4, 0); // from, to
    attribute(2, 2, 4); // start, heading
    attribute(3, 3, 6); // colour
    attribute(4, 3, 9); // size, alpha, halo
    gl2.bindVertexArray(null);
  }

  /** Uploads the fleet if it changed: each cab glides on from wherever it's drawn now. */
  private upload(now: number): void {
    const gl = this.gl!;
    if (this.data.length < this.marks.length * STRIDE) {
      this.data = new Float32Array(this.marks.length * STRIDE * 2);
    }
    const data = this.data;
    const [ox, oy] = [this.origin.x, this.origin.y];
    const seen = new Set<string>();
    this.marks.forEach((mark, i) => {
      seen.add(mark.id);
      let glide = this.glides.get(mark.id);
      if (!glide) {
        glide = { from: mark.at, to: mark.at, started: now };
        this.glides.set(mark.id, glide);
      } else if (glide.to[0] !== mark.at[0] || glide.to[1] !== mark.at[1]) {
        glide.from = glideAt(glide, now);
        glide.to = mark.at;
        glide.started = now;
      }
      const [r, g, b] = hexColor(mark.color);
      const o = i * STRIDE;
      data[o] = mercatorX(glide.from[0]) - ox;
      data[o + 1] = mercatorY(glide.from[1]) - oy;
      data[o + 2] = mercatorX(glide.to[0]) - ox;
      data[o + 3] = mercatorY(glide.to[1]) - oy;
      data[o + 4] = glide.started - this.epoch;
      data[o + 5] = mark.heading;
      data[o + 6] = r;
      data[o + 7] = g;
      data[o + 8] = b;
      data[o + 9] = mark.selected ? 13 : 9;
      data[o + 10] = mark.dim ? 0.45 : 1;
      data[o + 11] = mark.halo ? 1 : 0;
    });
    for (const id of this.glides.keys()) if (!seen.has(id)) this.glides.delete(id);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.instances!);
    gl.bufferData(gl.ARRAY_BUFFER, data.subarray(0, this.marks.length * STRIDE), gl.DYNAMIC_DRAW);
    this.count = this.marks.length;
    this.dirty = false;
  }

  render(gl: WebGLRenderingContext | WebGL2RenderingContext, options: CustomRenderMethodInput): void {
    const gl2 = gl as WebGL2RenderingContext;
    const map = this.map!;
    const now = performance.now();
    if (this.dirty) this.upload(now);
    if (this.count === 0) return;

    // The matrix with the origin folded in, in double precision.
    const m = options.defaultProjectionData.mainMatrix as unknown as ArrayLike<number>;
    const [ox, oy] = [this.origin.x, this.origin.y];
    const matrix = new Float32Array(16);
    for (let i = 0; i < 16; i++) matrix[i] = m[i];
    for (let row = 0; row < 4; row++) matrix[12 + row] = m[row] * ox + m[4 + row] * oy + m[12 + row];

    const canvas = map.getCanvas();
    const ratio = canvas.width / canvas.clientWidth;
    gl2.useProgram(this.program!);
    gl2.uniformMatrix4fv(this.uniforms.u_matrix, false, matrix);
    gl2.uniform2f(this.uniforms.u_viewport, canvas.width, canvas.height);
    gl2.uniform1f(this.uniforms.u_now, now - this.epoch);
    gl2.uniform1f(this.uniforms.u_bearing, map.getBearing());
    const zoom = map.getZoom();
    const zoomScale = zoom < 10 ? 0.7 : zoom > 16 ? 1.6 : 0.7 + ((zoom - 10) / 6) * 0.9;
    gl2.uniform1f(this.uniforms.u_scale, zoomScale * this.scale * ratio);
    gl2.bindVertexArray(this.vao!);
    gl2.enable(gl2.BLEND);
    gl2.blendFunc(gl2.ONE, gl2.ONE_MINUS_SRC_ALPHA);
    for (const pass of [1, 0]) {
      gl2.uniform1f(this.uniforms.u_pass, pass);
      gl2.drawArraysInstanced(gl2.TRIANGLE_STRIP, 0, 4, this.count);
    }
    gl2.bindVertexArray(null);
    // Keep drawing while any cab is still gliding.
    for (const glide of this.glides.values()) {
      if (now - glide.started < GLIDE_MS) {
        map.triggerRepaint();
        break;
      }
    }
  }
}

function glideAt(glide: Glide, now: number): LngLat {
  const t = Math.min(1, Math.max(0, (now - glide.started) / GLIDE_MS));
  return [glide.from[0] + (glide.to[0] - glide.from[0]) * t, glide.from[1] + (glide.to[1] - glide.from[1]) * t];
}
