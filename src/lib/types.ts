export interface FrameEvent {
  data: string; // base64-encoded RGBA
  width: number;
  height: number;
  min_val: number;
  max_val: number;
  min_pos: number;
  max_pos: number;
}

export interface DeviceInfo {
  serial_number: string;
  part_number: string;
  firmware_version: string;
  supports_radiometry: boolean;
  supports_hw_pseudo_color: boolean;
  width: number;
  height: number;
  fps: number;
}

export type Palette = "ironblack" | "rainbow" | "grayscale";

export type ConnectionState =
  | "disconnected"
  | "connecting"
  | "connected"
  | "streaming"
  | "error";

// Default Lepton sensor dimensions
export const DEFAULT_WIDTH = 160;
export const DEFAULT_HEIGHT = 120;
export const DEFAULT_FPS = 9;
