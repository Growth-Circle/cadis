/* eslint-disable react/no-unknown-property */
import { Line, useTexture } from "@react-three/drei";
import { useFrame } from "@react-three/fiber";
import { useMemo, useRef } from "react";
import * as THREE from "three";
import { avatarFragmentShader, avatarVertexShader } from "./arcAvatarShaders.js";
import type { ArcAvatarQuality, CadisAvatarMode } from "./arcAvatarTypes.js";

interface ArcAvatarProps {
  avatarTextureUrl: string;
  mode: CadisAvatarMode;
  amplitude: number;
  quality: ArcAvatarQuality;
}

const MODE_COLOR: Record<CadisAvatarMode, string> = {
  idle: "#35dfff",
  listening: "#55fff0",
  thinking: "#2da8ff",
  speaking: "#8be9ff",
  coding: "#8b5cff",
  error: "#ff4ed8",
};

const PARTICLES_BY_QUALITY: Record<ArcAvatarQuality, number> = {
  low: 160,
  medium: 320,
  high: 700,
};

function makeRingPoints(radius: number, start = 0, end = Math.PI * 2, segments = 96) {
  const points: [number, number, number][] = [];
  for (let i = 0; i <= segments; i += 1) {
    const t = start + ((end - start) * i) / segments;
    points.push([Math.cos(t) * radius, Math.sin(t) * radius, 0]);
  }
  return points;
}

function ParticleHalo({ mode, quality }: { mode: CadisAvatarMode; quality: ArcAvatarQuality }) {
  const pointsRef = useRef<THREE.Points>(null);
  const color = MODE_COLOR[mode];
  const count = PARTICLES_BY_QUALITY[quality];

  const positions = useMemo(() => {
    const arr = new Float32Array(count * 3);
    for (let i = 0; i < count; i += 1) {
      const angle = Math.random() * Math.PI * 2;
      const radius = 1.35 + Math.random() * 0.95;
      const jitter = (Math.random() - 0.5) * 0.22;
      arr[i * 3] = Math.cos(angle) * (radius + jitter);
      arr[i * 3 + 1] = Math.sin(angle) * (radius + jitter);
      arr[i * 3 + 2] = (Math.random() - 0.5) * 0.16;
    }
    return arr;
  }, [count]);

  useFrame(({ clock }) => {
    if (!pointsRef.current) return;
    const t = clock.elapsedTime;
    pointsRef.current.rotation.z = t * (mode === "thinking" ? 0.08 : 0.035);
  });

  return (
    <points ref={pointsRef} position={[0, 0, -0.08]}>
      <bufferGeometry>
        <bufferAttribute attach="attributes-position" args={[positions, 3]} />
      </bufferGeometry>
      <pointsMaterial
        color={color}
        size={quality === "high" ? 0.018 : 0.022}
        transparent
        opacity={mode === "idle" ? 0.42 : 0.68}
        depthWrite={false}
        blending={THREE.AdditiveBlending}
      />
    </points>
  );
}

function Reticle({ mode, amplitude }: { mode: CadisAvatarMode; amplitude: number }) {
  const groupRef = useRef<THREE.Group>(null);
  const color = MODE_COLOR[mode];

  useFrame(({ clock }) => {
    if (!groupRef.current) return;
    const t = clock.elapsedTime;
    groupRef.current.rotation.z = t * (mode === "thinking" ? -0.18 : -0.06);
    const s = 1 + Math.sin(t * 2.2) * 0.012 + amplitude * 0.025;
    groupRef.current.scale.setScalar(s);
  });

  const ring1 = useMemo(() => makeRingPoints(0.9, 0, Math.PI * 2, 128), []);
  const ring2 = useMemo(() => makeRingPoints(1.15, Math.PI * 0.12, Math.PI * 1.72, 96), []);
  const ring3 = useMemo(() => makeRingPoints(1.45, Math.PI * 0.84, Math.PI * 1.95, 80), []);
  const ring4 = useMemo(() => makeRingPoints(1.45, Math.PI * 0.05, Math.PI * 0.38, 40), []);

  return (
    <group ref={groupRef} position={[0, 0, 0.06]}>
      <Line points={ring1} color={color} transparent opacity={0.24} lineWidth={0.9} />
      <Line points={ring2} color={color} transparent opacity={0.42} lineWidth={1.2} />
      <Line points={ring3} color={color} transparent opacity={0.26} lineWidth={0.9} />
      <Line points={ring4} color={color} transparent opacity={0.28} lineWidth={0.9} />
      {[
        [0, 0.98, 0],
        [0.98, 0, 0],
        [0, -0.98, 0],
        [-0.98, 0, 0],
      ].map((position, index) => (
        <group key={index} position={position as [number, number, number]}>
          <mesh>
            <sphereGeometry args={[0.035 + amplitude * 0.035, 16, 16]} />
            <meshBasicMaterial color={color} transparent opacity={0.92} />
          </mesh>
        </group>
      ))}
      <Line points={[[0, 0.2, 0], [0, 0.92, 0]]} color={color} transparent opacity={0.24} lineWidth={0.8} />
      <Line points={[[0.2, 0, 0], [0.92, 0, 0]]} color={color} transparent opacity={0.24} lineWidth={0.8} />
      <Line points={[[0, -0.2, 0], [0, -0.92, 0]]} color={color} transparent opacity={0.24} lineWidth={0.8} />
      <Line points={[[-0.2, 0, 0], [-0.92, 0, 0]]} color={color} transparent opacity={0.24} lineWidth={0.8} />
    </group>
  );
}

function FaceExpressionOverlay({ mode, amplitude }: { mode: CadisAvatarMode; amplitude: number }) {
  const groupRef = useRef<THREE.Group>(null);
  const leftEyeRef = useRef<THREE.Mesh>(null);
  const rightEyeRef = useRef<THREE.Mesh>(null);
  const mouthGlowRef = useRef<THREE.Mesh>(null);
  const mouthGroupRef = useRef<THREE.Group>(null);
  const color = MODE_COLOR[mode];
  const smilePoints = useMemo(
    () => [
      [-0.26, 0.018, 0.01],
      [-0.16, -0.018, 0.01],
      [0, -0.034, 0.01],
      [0.16, -0.018, 0.01],
      [0.26, 0.018, 0.01],
    ] as [number, number, number][],
    [],
  );

  useFrame(({ clock }) => {
    const t = clock.elapsedTime;
    const speaking = mode === "speaking";
    const listening = mode === "listening";
    const active = speaking || listening;
    const gazeX = Math.sin(t * 0.72) * 0.018 + (listening ? Math.sin(t * 1.35) * 0.01 : 0);
    const gazeY = Math.sin(t * 0.48 + 1.2) * 0.01;
    const blinkPhase = t % 4.8;
    const blink = blinkPhase > 4.58 ? 0.18 : blinkPhase > 4.48 ? 0.42 : 1;
    const eyePulse = 1 + Math.sin(t * (active ? 5.2 : 2.1)) * (active ? 0.07 : 0.025);
    const mouthOpen = active
      ? 0.34 + Math.max(amplitude, 0.12) * 1.15 + Math.max(0, Math.sin(t * 9.5)) * 0.26
      : 0.12 + Math.max(0, Math.sin(t * 1.8)) * 0.03;

    if (groupRef.current) {
      groupRef.current.position.x = gazeX * 0.35;
      groupRef.current.position.y = gazeY * 0.3;
    }
    for (const eye of [leftEyeRef.current, rightEyeRef.current]) {
      if (!eye) continue;
      eye.position.x += (gazeX - (eye.userData.gazeX ?? 0)) * 0.18;
      eye.position.y += (gazeY - (eye.userData.gazeY ?? 0)) * 0.18;
      eye.userData.gazeX = eye.position.x;
      eye.userData.gazeY = eye.position.y;
      eye.scale.set(eyePulse, blink, 1);
    }
    if (mouthGroupRef.current) {
      mouthGroupRef.current.scale.set(1 + mouthOpen * 0.08, 1 + mouthOpen * 0.12, 1);
    }
    const mouthMaterial = mouthGlowRef.current?.material as THREE.MeshBasicMaterial | undefined;
    if (mouthGlowRef.current && mouthMaterial) {
      mouthGlowRef.current.scale.set(1.25, 0.14 + mouthOpen * 0.42, 1);
      mouthMaterial.opacity = speaking ? 0.24 + mouthOpen * 0.18 : 0.08 + mouthOpen * 0.12;
    }
  });

  return (
    <group ref={groupRef} position={[0, 0, 0.16]}>
      {[
        [-0.29, 0.28, 0],
        [0.29, 0.3, 0],
      ].map((position, index) => (
        <group key={index} position={position as [number, number, number]}>
          <mesh ref={index === 0 ? leftEyeRef : rightEyeRef}>
            <sphereGeometry args={[0.024, 18, 18]} />
            <meshBasicMaterial
              color="#a8ffff"
              transparent
              opacity={0.68}
              blending={THREE.AdditiveBlending}
              depthWrite={false}
            />
          </mesh>
          <mesh position={[0.018, 0.016, 0.012]}>
            <sphereGeometry args={[0.008, 12, 12]} />
            <meshBasicMaterial
              color="#ffffff"
              transparent
              opacity={0.76}
              blending={THREE.AdditiveBlending}
              depthWrite={false}
            />
          </mesh>
        </group>
      ))}
      <group ref={mouthGroupRef} position={[0, -0.43, 0]}>
        <mesh ref={mouthGlowRef} position={[0, -0.018, -0.01]}>
          <circleGeometry args={[0.16, 36]} />
          <meshBasicMaterial
            color={color}
            transparent
            opacity={0.14}
            blending={THREE.AdditiveBlending}
            depthWrite={false}
          />
        </mesh>
        <Line points={smilePoints} color="#dfffff" transparent opacity={0.52} lineWidth={1.8} />
        <Line points={smilePoints} color={color} transparent opacity={0.62} lineWidth={0.9} />
      </group>
    </group>
  );
}

export function ArcAvatar({ avatarTextureUrl, mode, amplitude, quality }: ArcAvatarProps) {
  const materialRef = useRef<THREE.ShaderMaterial>(null);
  const groupRef = useRef<THREE.Group>(null);
  const texture = useTexture(avatarTextureUrl);

  texture.colorSpace = THREE.SRGBColorSpace;
  texture.minFilter = THREE.LinearMipmapLinearFilter;
  texture.magFilter = THREE.LinearFilter;
  texture.anisotropy = 8;

  const modeColor = useMemo(() => new THREE.Color(MODE_COLOR[mode]), [mode]);

  const uniforms = useMemo(
    () => ({
      uTexture: { value: texture },
      uModeColor: { value: modeColor },
      uTime: { value: 0 },
      uOpacity: { value: 1.0 },
      uAmplitude: { value: amplitude },
      uBackgroundCutoff: { value: 0.014 },
      uGlow: { value: mode === "speaking" ? 1.0 : 0.68 },
    }),
    [texture, modeColor, amplitude, mode],
  );

  useFrame(({ clock }) => {
    const t = clock.elapsedTime;
    const mat = materialRef.current;
    if (mat) {
      mat.uniforms.uTime.value = t;
      mat.uniforms.uAmplitude.value = amplitude;
      mat.uniforms.uModeColor.value = modeColor;
      mat.uniforms.uGlow.value =
        mode === "speaking"
          ? 0.9 + amplitude * 1.1
          : mode === "thinking"
            ? 0.72
            : 0.58;
    }
    if (groupRef.current) {
      const breathe = 1 + Math.sin(t * 1.35) * 0.012 + amplitude * 0.025;
      groupRef.current.scale.setScalar(breathe);
    }
  });

  return (
    <group ref={groupRef}>
      <ParticleHalo mode={mode} quality={quality} />
      <mesh position={[0, 0.02, 0]}>
        <planeGeometry args={[3.42, 3.42, 96, 96]} />
        <shaderMaterial
          ref={materialRef}
          uniforms={uniforms}
          vertexShader={avatarVertexShader}
          fragmentShader={avatarFragmentShader}
          transparent
          depthWrite={false}
          blending={THREE.NormalBlending}
        />
      </mesh>
      <FaceExpressionOverlay mode={mode} amplitude={amplitude} />
      <Reticle mode={mode} amplitude={amplitude} />
    </group>
  );
}
