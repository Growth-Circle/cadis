/* eslint-disable react/no-unknown-property */
import { PerspectiveCamera, Preload } from "@react-three/drei";
import { Canvas } from "@react-three/fiber";
import { Suspense } from "react";
import { ArcAvatar } from "./ArcAvatar.js";
import type { ArcAvatarSceneProps } from "./arcAvatarTypes.js";

export function ArcAvatarScene({
  avatarTextureUrl,
  mode = "idle",
  amplitude = 0,
  quality = "medium",
  className = "",
}: ArcAvatarSceneProps) {
  return (
    <div className={`cadis-arc-avatar-root ${className}`} data-mode={mode}>
      <Canvas
        dpr={quality === "high" ? [1, 2] : [1, 1.5]}
        gl={{
          alpha: true,
          antialias: quality !== "low",
          powerPreference: "high-performance",
        }}
        camera={{ position: [0, 0, 5.6], fov: 36 }}
      >
        <PerspectiveCamera makeDefault position={[0, 0, 5.6]} fov={36} />
        <ambientLight intensity={0.65} />
        <Suspense fallback={null}>
          <ArcAvatar
            avatarTextureUrl={avatarTextureUrl}
            mode={mode}
            amplitude={Math.max(0, Math.min(1, amplitude))}
            quality={quality}
          />
          <Preload all />
        </Suspense>
      </Canvas>
    </div>
  );
}
