// The 3D view: a slab of tiles hanging in open air, seen from above and
// behind. three.js picks WebGPU when the browser has it and WebGL2 when
// it does not; nothing here depends on which.

import * as THREE from "three/webgpu";
import { color, mix, screenUV, smoothstep } from "three/tsl";

import { palette, seatColours } from "../palette";
import { TileState, type View } from "../view";
import { Motion, type Cue, type Sample } from "./motion";
import type { Stage } from "./stage";

const TILE = 2;
const PLAYER_RADIUS = 0.4;
const PLAYER_HALF_HEIGHT = 0.35;
const MAX_PROPS = 32;
const RING_POOL = 16;

interface Ring {
  mesh: THREE.Mesh;
  material: THREE.MeshBasicMaterial;
  age: number;
  life: number;
  from: number;
  to: number;
}

interface Figure {
  root: THREE.Group;
  body: THREE.Mesh;
  bodyMaterial: THREE.MeshStandardMaterial;
  shield: THREE.Mesh;
  flash: number;
}

function makeFigure(seat: number): Figure {
  const tint = new THREE.Color(seatColours[seat] ?? "#ffffff");
  const root = new THREE.Group();

  const bodyMaterial = new THREE.MeshStandardMaterial({ color: tint, roughness: 0.55 });
  const body = new THREE.Mesh(
    new THREE.CapsuleGeometry(PLAYER_RADIUS, PLAYER_HALF_HEIGHT * 2, 6, 20),
    bodyMaterial,
  );
  body.castShadow = true;
  root.add(body);

  // A dark visor on the side the player faces, so heading reads at a
  // glance from above.
  const visor = new THREE.Mesh(
    new THREE.SphereGeometry(0.3, 16, 12),
    new THREE.MeshStandardMaterial({ color: "#14212b", roughness: 0.2 }),
  );
  visor.scale.set(0.55, 0.5, 1);
  visor.position.set(0.26, 0.3, 0);
  body.add(visor);

  const shield = new THREE.Mesh(
    new THREE.SphereGeometry(0.85, 20, 14),
    new THREE.MeshBasicMaterial({
      color: tint,
      transparent: true,
      opacity: 0.22,
      depthWrite: false,
    }),
  );
  root.add(shield);

  return { root, body, bodyMaterial, shield, flash: 0 };
}

export interface SceneOptions {
  /** Skip WebGPU even where the browser offers it. */
  forceWebGL: boolean;
  /** The GPU device went away; this stage will draw nothing more. */
  onLost: () => void;
}

export async function createScene(canvas: HTMLCanvasElement, options: SceneOptions): Promise<Stage> {
  const renderer = new THREE.WebGPURenderer({
    canvas,
    antialias: true,
    forceWebGL: options.forceWebGL,
  });
  await renderer.init();
  let lost = false;
  renderer.onDeviceLost = () => {
    if (!lost) {
      lost = true;
      options.onLost();
    }
  };
  renderer.shadowMap.enabled = true;
  renderer.toneMapping = THREE.ACESFilmicToneMapping;

  const scene = new THREE.Scene();
  // Open sky that darkens toward the drop. Falling tiles fade into it.
  scene.backgroundNode = mix(
    color(palette.skyBottom),
    color(palette.skyTop),
    smoothstep(0.15, 1.0, screenUV.y.oneMinus()),
  );
  scene.fog = new THREE.Fog(palette.skyBottom, 34, 62);

  scene.add(new THREE.HemisphereLight("#cfe8f2", "#27414d", 1.1));
  const sun = new THREE.DirectionalLight("#fff1d6", 2.6);
  sun.position.set(-9, 20, 7);
  sun.castShadow = true;
  sun.shadow.mapSize.set(2048, 2048);
  sun.shadow.camera.left = -16;
  sun.shadow.camera.right = 16;
  sun.shadow.camera.top = 16;
  sun.shadow.camera.bottom = -16;
  sun.shadow.camera.near = 1;
  sun.shadow.camera.far = 60;
  sun.shadow.bias = -0.0008;
  scene.add(sun);

  const camera = new THREE.PerspectiveCamera(38, 1, 0.5, 120);
  const focus = new THREE.Vector3();
  let shake = 0;

  let tiles: THREE.InstancedMesh | null = null;
  const tileColour = new THREE.Color(palette.tile);
  const warningColour = new THREE.Color(palette.warning);

  const crates = new THREE.InstancedMesh(
    new THREE.BoxGeometry(0.8, 0.8, 0.8),
    new THREE.MeshStandardMaterial({ color: palette.crate, roughness: 0.8 }),
    MAX_PROPS,
  );
  const balls = new THREE.InstancedMesh(
    new THREE.SphereGeometry(0.45, 20, 14),
    new THREE.MeshStandardMaterial({ color: palette.ball, roughness: 0.35 }),
    MAX_PROPS,
  );
  for (const mesh of [crates, balls]) {
    mesh.castShadow = true;
    mesh.receiveShadow = true;
    mesh.frustumCulled = false;
    mesh.count = 0;
    scene.add(mesh);
  }

  const figures: Figure[] = [];
  const rings: Ring[] = [];
  const ringGeometry = new THREE.RingGeometry(0.86, 1, 40);
  ringGeometry.rotateX(-Math.PI / 2);
  for (let i = 0; i < RING_POOL; i++) {
    const material = new THREE.MeshBasicMaterial({
      transparent: true,
      opacity: 0,
      depthWrite: false,
      side: THREE.DoubleSide,
    });
    const mesh = new THREE.Mesh(ringGeometry, material);
    mesh.visible = false;
    scene.add(mesh);
    rings.push({ mesh, material, age: 0, life: 0, from: 0, to: 0 });
  }

  const burst = (x: number, y: number, z: number, tint: string, from: number, to: number, life: number) => {
    const ring = rings.find((r) => !r.mesh.visible) ?? rings[0]!;
    ring.mesh.position.set(x, y, z);
    ring.material.color.set(tint);
    ring.age = 0;
    ring.life = life;
    ring.from = from;
    ring.to = to;
    ring.mesh.visible = true;
  };

  const motion = new Motion();
  const matrix = new THREE.Matrix4();
  const position = new THREE.Vector3();
  const rotation = new THREE.Quaternion();
  const scale = new THREE.Vector3(1, 1, 1);
  const tint = new THREE.Color();
  let lastFrameAt = 0;
  let me = 0;

  const buildTiles = (view: View) => {
    if (tiles) {
      scene.remove(tiles);
      tiles.dispose();
    }
    const geometry = new THREE.BoxGeometry(TILE - 0.08, 0.5, TILE - 0.08);
    const material = new THREE.MeshStandardMaterial({ color: "#ffffff", roughness: 0.9 });
    tiles = new THREE.InstancedMesh(geometry, material, view.tiles.length);
    tiles.receiveShadow = true;
    tiles.castShadow = true;
    tiles.frustumCulled = false;
    scene.add(tiles);
  };

  const react = (cue: Cue) => {
    const seatTint = "seat" in cue ? (seatColours[cue.seat] ?? "#ffffff") : palette.warning;
    switch (cue.kind) {
      case "shove":
        burst(cue.x + Math.cos(cue.turn) * 0.9, cue.y, cue.z + Math.sin(cue.turn) * 0.9, palette.ink, 0.3, 1.5, 0.18);
        break;
      case "dash":
        burst(cue.x, cue.y - 0.6, cue.z, seatTint, 0.5, 1.1, 0.25);
        break;
      case "hit": {
        const figure = figures[cue.seat];
        if (figure) figure.flash = 1;
        if (cue.seat === me) shake = Math.max(shake, 0.25);
        break;
      }
      case "ring-out":
        burst(cue.x, Math.max(cue.y, -1), cue.z, seatTint, 0.6, 5, 0.6);
        shake = Math.max(shake, cue.seat === me ? 0.7 : 0.3);
        break;
      case "tile-fell":
        burst(cue.x, 0.3, cue.z, palette.warning, 0.9, 1.6, 0.3);
        break;
    }
  };

  const place = (sample: Sample, seconds: number, dt: number) => {
    const view = sample.view;
    if (!tiles || tiles.count !== view.tiles.length) buildTiles(view);
    while (figures.length < view.players.length) {
      const figure = makeFigure(figures.length);
      figures.push(figure);
      scene.add(figure.root);
    }

    sample.tiles.forEach((tile, i) => {
      const state = tile.now.state;
      const gone = state === TileState.gone;
      position.set(tile.x, tile.y, tile.z);
      tint.copy(tileColour);
      // Alternate a slightly darker tile so speed over the floor is visible.
      const grid = Math.round(Math.sqrt(view.tiles.length));
      if (((i % grid) + Math.floor(i / grid)) % 2 === 1) tint.multiplyScalar(0.9);
      if (state === TileState.cracking) {
        const pulse = 0.5 + 0.5 * Math.sin(seconds * 22);
        tint.lerp(warningColour, 0.45 + 0.4 * pulse);
        position.x += Math.sin(seconds * 61 + i) * 0.025;
        position.z += Math.cos(seconds * 53 + i * 2) * 0.025;
      } else if (state === TileState.falling) {
        tint.lerp(warningColour, 0.25);
      }
      rotation.set(tile.qx, tile.qy, tile.qz, tile.qw);
      scale.setScalar(gone ? 0 : 1);
      tiles!.setMatrixAt(i, matrix.compose(position, rotation, scale));
      tiles!.setColorAt(i, tint);
    });
    tiles!.instanceMatrix.needsUpdate = true;
    if (tiles!.instanceColor) tiles!.instanceColor.needsUpdate = true;

    scale.setScalar(1);
    let crateCount = 0;
    let ballCount = 0;
    for (const prop of sample.props) {
      position.set(prop.x, prop.y, prop.z);
      rotation.set(prop.qx, prop.qy, prop.qz, prop.qw);
      matrix.compose(position, rotation, scale);
      if (prop.now.round) {
        if (ballCount < MAX_PROPS) balls.setMatrixAt(ballCount++, matrix);
      } else if (crateCount < MAX_PROPS) {
        crates.setMatrixAt(crateCount++, matrix);
      }
    }
    crates.count = crateCount;
    balls.count = ballCount;
    crates.instanceMatrix.needsUpdate = true;
    balls.instanceMatrix.needsUpdate = true;

    sample.players.forEach((player, seat) => {
      const figure = figures[seat]!;
      const state = player.now;
      figure.root.visible = state.inPlay;
      figure.root.position.set(player.x, player.y, player.z);
      figure.body.rotation.y = -player.turn;
      // Stretch when rising or falling fast, lean into a stun.
      const stretch = 1 + Math.min(0.25, Math.abs(state.vy) * 0.018);
      figure.body.scale.set(1 / Math.sqrt(stretch), stretch, 1 / Math.sqrt(stretch));
      figure.body.rotation.z = state.stunned ? 0.35 * Math.sin(seconds * 30) : 0;
      figure.shield.visible = state.shielded;
      figure.shield.scale.setScalar(1 + 0.05 * Math.sin(seconds * 6));
      figure.flash = Math.max(0, figure.flash - dt * 6);
      figure.bodyMaterial.emissive.set("#ffffff").multiplyScalar(figure.flash * 0.8);
    });
    for (let seat = view.players.length; seat < figures.length; seat++) {
      figures[seat]!.root.visible = false;
    }

    // The camera leans toward the local player without leaving the arena.
    const mine = sample.players[view.me];
    const lean = mine && mine.now.inPlay ? 0.22 : 0;
    focus.lerp(position.set((mine?.x ?? 0) * lean, 0, (mine?.z ?? 0) * lean + 1), 1 - Math.exp(-dt * 4));
  };

  const frameCamera = (seconds: number, dt: number) => {
    const grid = tiles ? Math.round(Math.sqrt(tiles.count)) : 9;
    const span = (grid * TILE) / 2 + 2.5;
    // Pull back until the arena fits the narrower dimension.
    const vertical = (camera.fov * Math.PI) / 360;
    const fitHeight = (span * 0.92) / Math.tan(vertical);
    const fitWidth = span / (Math.tan(vertical) * camera.aspect);
    const distance = Math.max(fitHeight, fitWidth);
    shake = Math.max(0, shake - dt * 2.2);
    const jolt = shake * shake;
    camera.position.set(
      focus.x + Math.sin(seconds * 47) * jolt * 0.5,
      distance * 0.82 + Math.sin(seconds * 53) * jolt * 0.5,
      focus.z + distance * 0.57,
    );
    camera.lookAt(focus.x, 0, focus.z);
  };

  const resize = () => {
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    if (width === 0 || height === 0) return;
    renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
    renderer.setSize(width, height, false);
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
  };
  resize();

  const backend = renderer.backend as { isWebGPUBackend?: boolean };

  return {
    kind: backend.isWebGPUBackend ? "webgpu" : "webgl",
    push(view, corrected, nowMs) {
      me = view.me;
      motion.push(view, corrected, nowMs);
    },
    frame(nowMs) {
      if (lost) return [];
      let cues: Cue[] = [];
      const dt = Math.min(0.1, (nowMs - lastFrameAt) / 1000);
      lastFrameAt = nowMs;
      const seconds = nowMs / 1000;
      const sample = motion.sample(nowMs);
      if (sample) {
        cues = motion.takeCues();
        cues.forEach(react);
        place(sample, seconds, dt);
      }
      for (const ring of rings) {
        if (!ring.mesh.visible) continue;
        ring.age += dt;
        const t = ring.age / ring.life;
        if (t >= 1) {
          ring.mesh.visible = false;
          continue;
        }
        const eased = 1 - (1 - t) * (1 - t);
        ring.mesh.scale.setScalar(ring.from + (ring.to - ring.from) * eased);
        ring.material.opacity = (1 - t) * 0.9;
      }
      frameCamera(seconds, dt);
      renderer.render(scene, camera);
      return cues;
    },
    resize,
  };
}
