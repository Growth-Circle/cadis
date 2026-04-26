export type CadisAvatarMode =
  | "idle"
  | "listening"
  | "thinking"
  | "speaking"
  | "coding"
  | "error";

export type ArcAvatarQuality = "low" | "medium" | "high";

export interface ArcAvatarSceneProps {
  avatarTextureUrl: string;
  mode?: CadisAvatarMode;
  amplitude?: number;
  quality?: ArcAvatarQuality;
  className?: string;
}
