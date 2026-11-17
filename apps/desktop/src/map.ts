// Hand-written WebGL2 instanced renderer (D-04). React never touches this path.

export interface Inst { x: number; y: number; w: number; h: number; id: number; depth: number; kind: number; cls: number; age: number; }

const STRIDE = 24;
export const KIND_DIR = 1, KIND_SMALL = 2, KIND_LINK = 4, KIND_RECLAIM = 8;
export type Lens = "none" | "type" | "age" | "reclaim";
const LENS_ID: Record<Lens, number> = { none: 0, type: 1, age: 2, reclaim: 3 };

const VS = `#version 300 es
layout(location=0) in vec2 quad;
layout(location=1) in vec4 rect;
layout(location=2) in uvec4 meta;
layout(location=3) in uint nid;
layout(location=4) in float dim;
uniform vec2 res; uniform vec4 cam;
out vec2 uv; flat out vec2 size; flat out uvec4 m; flat out uint id; flat out float d;
void main(){
  vec2 p = rect.xy + quad*rect.zw;
  vec2 s = (p - cam.xy)*cam.zw;
  gl_Position = vec4(s.x/res.x*2.-1., 1.-s.y/res.y*2., 0., 1.);
  uv = quad; size = rect.zw*cam.zw; m = meta; id = nid; d = dim;
}`;

const FS = `#version 300 es
precision highp float;
in vec2 uv; flat in vec2 size; flat in uvec4 m; flat in uint id; flat in float d;
uniform int lens; uniform uint hover; uniform uint selected; uniform vec3 pal[9]; uniform float reveal;
out vec4 c;
void main(){
  uint depth=m.x, kind=m.y, cls=m.z, age=m.w;
  bool dir=(kind&1u)!=0u, small=(kind&2u)!=0u, link=(kind&4u)!=0u, marked=(kind&8u)!=0u;
  float dd=float(depth);
  vec3 col;
  // Monochrome relief: files catch light, folders recede, depth lifts both.
  float L = dir ? 0.035+0.012*dd : min(0.30, 0.13+0.022*dd);
  vec2 px = uv*size;
  float light = 1.22-0.44*(uv.x*0.45+uv.y*0.55);
  col = vec3(L*(dir?1.0:light));
  if(!dir && lens==1) col = pal[cls]*light;
  if(lens==2){ float t=float(age)/7.0; col = dir ? col : mix(vec3(.46),vec3(.05),t)*light; }
  if(lens==3){ col = marked ? vec3(1.0,.706,.329)*(dir?0.22:0.62)*light : col*0.55; }
  if(small){
    // Hatching on thin cells reads as stray wedges, so those stay flat.
    float h = min(size.x,size.y)<14.0 ? 0.4 : step(0.5,fract((px.x+px.y)/6.0));
    col = vec3(0.06+0.035*h);
  }
  if(link){ float h = min(size.x,size.y)<14.0 ? 0.5 : step(0.5,fract((px.x-px.y)/10.0)); col = mix(col,vec3(.22),.5*h); }
  col *= d;
  // 1px black gutter on every side defines structure.
  float e = min(min(px.x,size.x-px.x),min(px.y,size.y-px.y));
  if(size.x>2.5 && size.y>2.5 && e<0.75) col=vec3(0.);
  if(id==selected && !small){ if(e<2.0) col=vec3(.95); else col*=1.45; }
  else if(id==hover && !small){ if(e<1.25) col=vec3(.75); else col+=vec3(.05); }
  c=vec4(col*reveal,1.);
}`;

function compile(gl: WebGL2RenderingContext, ty: number, src: string) {
  const s = gl.createShader(ty)!;
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? "shader");
  return s;
}

// Distinct mid-luminance palette for the Type lens, labelled in the legend.
const PALETTE = [
  [.24, .24, .24], [.86, .33, .36], [.27, .62, .78], [.76, .47, .86], [.86, .79, .42],
  [.91, .56, .26], [.40, .73, .47], [.34, .46, .88], [.62, .62, .60],
];

export class MapRenderer {
  private gl: WebGL2RenderingContext;
  private u: Record<string, WebGLUniformLocation | null> = {};
  private instBuf: WebGLBuffer;
  private dimBuf: WebGLBuffer;
  private vao: WebGLVertexArrayObject;
  data = new DataView(new ArrayBuffer(0));
  count = 0;
  private hover = 0xffffffff;
  private selected = 0xffffffff;
  private lens: Lens = "none";
  private cam = [0, 0, 1, 1];
  private anim: { from: number[]; to: number[]; t0: number; ms: number; done?: () => void } | null = null;
  private revealT = 1;
  private revealAnim: { t0: number; ms: number } | null = null;
  private raf = 0;
  private dirty = true;
  private matches: Set<number> | null = null;
  width = 1;
  height = 1;
  onFrame: (() => void) | null = null;
  frameLog: number[] | null = null;
  private lastFrame = 0;

  constructor(private canvas: HTMLCanvasElement) {
    const gl = canvas.getContext("webgl2", { antialias: false, alpha: false, powerPreference: "high-performance" });
    if (!gl) throw new Error("WebGL2 is unavailable");
    this.gl = gl;
    const p = gl.createProgram()!;
    gl.attachShader(p, compile(gl, gl.VERTEX_SHADER, VS));
    gl.attachShader(p, compile(gl, gl.FRAGMENT_SHADER, FS));
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) ?? "link");
    gl.useProgram(p);
    for (const n of ["res", "cam", "lens", "hover", "selected", "pal", "reveal"]) this.u[n] = gl.getUniformLocation(p, n);
    gl.uniform3fv(this.u.pal, new Float32Array(PALETTE.flat()));

    this.vao = gl.createVertexArray()!;
    gl.bindVertexArray(this.vao);
    const quad = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, quad);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);

    this.instBuf = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.instBuf);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 4, gl.FLOAT, false, STRIDE, 0);
    gl.vertexAttribDivisor(1, 1);
    gl.enableVertexAttribArray(3);
    gl.vertexAttribIPointer(3, 1, gl.UNSIGNED_INT, STRIDE, 16);
    gl.vertexAttribDivisor(3, 1);
    gl.enableVertexAttribArray(2);
    gl.vertexAttribIPointer(2, 4, gl.UNSIGNED_BYTE, STRIDE, 20);
    gl.vertexAttribDivisor(2, 1);

    this.dimBuf = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.dimBuf);
    gl.enableVertexAttribArray(4);
    gl.vertexAttribPointer(4, 1, gl.FLOAT, false, 0, 0);
    gl.vertexAttribDivisor(4, 1);
  }

  resize(w: number, h: number) {
    const dpr = window.devicePixelRatio || 1;
    this.width = w;
    this.height = h;
    this.canvas.width = Math.max(1, Math.round(w * dpr));
    this.canvas.height = Math.max(1, Math.round(h * dpr));
    this.gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    this.invalidate();
  }

  setData(buf: ArrayBuffer) {
    const gl = this.gl;
    this.data = new DataView(buf);
    this.count = buf.byteLength / STRIDE;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.instBuf);
    gl.bufferData(gl.ARRAY_BUFFER, buf, gl.DYNAMIC_DRAW);
    this.updateDim();
    this.invalidate();
  }

  inst(i: number): Inst {
    const d = this.data, o = i * STRIDE;
    return {
      x: d.getFloat32(o, true), y: d.getFloat32(o + 4, true), w: d.getFloat32(o + 8, true), h: d.getFloat32(o + 12, true),
      id: d.getUint32(o + 16, true), depth: d.getUint8(o + 20), kind: d.getUint8(o + 21), cls: d.getUint8(o + 22), age: d.getUint8(o + 23),
    };
  }

  setMatches(m: Set<number> | null) {
    this.matches = m;
    this.updateDim();
    this.invalidate();
  }

  private updateDim() {
    const a = new Float32Array(this.count).fill(1);
    if (this.matches) {
      for (let i = 0; i < this.count; i++) {
        const id = this.data.getUint32(i * STRIDE + 16, true);
        const kind = this.data.getUint8(i * STRIDE + 21);
        a[i] = this.matches.has(id) ? 2.2 : kind & KIND_DIR ? 0.7 : 0.22;
      }
    }
    const gl = this.gl;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.dimBuf);
    gl.bufferData(gl.ARRAY_BUFFER, a, gl.DYNAMIC_DRAW);
  }

  setHover(id: number | null) { const v = id ?? 0xffffffff; if (v !== this.hover) { this.hover = v; this.invalidate(); } }
  setSelected(id: number | null) { const v = id ?? 0xffffffff; if (v !== this.selected) { this.selected = v; this.invalidate(); } }
  setLens(l: Lens) { this.lens = l; this.invalidate(); }

  /** Deepest instance at a point. Parents precede children, so scan backwards. */
  pick(px: number, py: number): number {
    const [cx, cy, sx, sy] = this.cam;
    const x = px / sx + cx, y = py / sy + cy, d = this.data;
    for (let i = this.count - 1; i >= 0; i--) {
      const o = i * STRIDE, ix = d.getFloat32(o, true);
      if (x < ix || x >= ix + d.getFloat32(o + 8, true)) continue;
      const iy = d.getFloat32(o + 4, true);
      if (y >= iy && y < iy + d.getFloat32(o + 12, true)) return i;
    }
    return -1;
  }

  findById(id: number): number {
    for (let i = 0; i < this.count; i++) {
      if (this.data.getUint32(i * STRIDE + 16, true) === id && !(this.data.getUint8(i * STRIDE + 21) & KIND_SMALL)) return i;
    }
    return -1;
  }

  resetCam() { this.anim = null; this.cam = [0, 0, 1, 1]; this.invalidate(); }

  camera(): number[] { return this.cam; }

  /** Interruptible camera move; a new call retargets from the current position. */
  flyTo(r: { x: number; y: number; w: number; h: number }, ms: number, done?: () => void) {
    this.flyCam([r.x, r.y, this.width / Math.max(r.w, 1), this.height / Math.max(r.h, 1)], ms, done);
  }

  flyFrom(r: { x: number; y: number; w: number; h: number }, ms: number) {
    this.cam = [r.x, r.y, this.width / Math.max(r.w, 1), this.height / Math.max(r.h, 1)];
    this.flyCam([0, 0, 1, 1], ms);
  }

  private flyCam(to: number[], ms: number, done?: () => void) {
    if (ms <= 0) { this.cam = to; this.anim = null; this.invalidate(); done?.(); return; }
    this.anim = { from: [...this.cam], to, t0: performance.now(), ms, done };
    this.invalidate();
  }

  playReveal(ms: number) {
    if (ms <= 0) return;
    this.revealAnim = { t0: performance.now(), ms };
    this.invalidate();
  }

  invalidate() {
    this.dirty = true;
    if (!this.raf) this.raf = requestAnimationFrame(this.tick);
  }

  private tick = (now: number) => {
    this.raf = 0;
    if (this.frameLog && this.lastFrame) this.frameLog.push(now - this.lastFrame);
    this.lastFrame = now;
    let more = false;
    if (this.anim) {
      const a = this.anim, t = Math.min(1, (now - a.t0) / a.ms), e = t < .5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
      // Scales interpolate geometrically so zoom speed feels uniform.
      const sx = a.from[2] * Math.pow(a.to[2] / a.from[2], e), sy = a.from[3] * Math.pow(a.to[3] / a.from[3], e);
      this.cam = [a.from[0] + (a.to[0] - a.from[0]) * e, a.from[1] + (a.to[1] - a.from[1]) * e, sx, sy];
      if (t >= 1) { const d = a.done; this.anim = null; d?.(); } else more = true;
      this.dirty = true;
    }
    if (this.revealAnim) {
      const t = Math.min(1, (now - this.revealAnim.t0) / this.revealAnim.ms);
      this.revealT = 1 - Math.pow(1 - t, 4);
      if (t >= 1) this.revealAnim = null; else more = true;
      this.dirty = true;
    }
    if (this.dirty) this.draw();
    if (more || this.anim) this.raf = requestAnimationFrame(this.tick);
    else this.lastFrame = 0;
  };

  private draw() {
    const gl = this.gl;
    this.dirty = false;
    gl.clearColor(0, 0, 0, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.uniform2f(this.u.res, this.width, this.height);
    gl.uniform4f(this.u.cam, this.cam[0], this.cam[1], this.cam[2], this.cam[3]);
    gl.uniform1i(this.u.lens, LENS_ID[this.lens]);
    gl.uniform1ui(this.u.hover, this.hover);
    gl.uniform1ui(this.u.selected, this.selected);
    gl.uniform1f(this.u.reveal, this.revealT);
    gl.bindVertexArray(this.vao);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, this.count);
    this.onFrame?.();
  }

  destroy() { cancelAnimationFrame(this.raf); this.gl.getExtension("WEBGL_lose_context")?.loseContext(); }
}

export const LENS_LEGEND = PALETTE;
