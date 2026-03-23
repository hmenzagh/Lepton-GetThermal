import { useEffect, useRef, useCallback } from "react";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { FrameEvent } from "../lib/types";

export interface FrameStats {
  minVal: number;
  maxVal: number;
  minPos: number;
  maxPos: number;
  width: number;
  height: number;
}

export function useFrameStream(
  canvasRef: React.RefObject<HTMLCanvasElement | null>,
  onStats?: (stats: FrameStats) => void,
  onDisconnect?: () => void
) {
  const unlistenRef = useRef<UnlistenFn | null>(null);
  const watchdogRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const resetWatchdog = useCallback(() => {
    if (watchdogRef.current) clearTimeout(watchdogRef.current);
    if (onDisconnect) {
      // If no frame arrives within 3 seconds, consider device disconnected
      watchdogRef.current = setTimeout(() => {
        onDisconnect();
      }, 3000);
    }
  }, [onDisconnect]);

  const start = useCallback(async () => {
    if (unlistenRef.current) return;

    unlistenRef.current = await listen<FrameEvent>("thermal-frame", (event) => {
      const canvas = canvasRef.current;
      if (!canvas) return;

      const { data, width, height, min_val, max_val, min_pos, max_pos } = event.payload;

      resetWatchdog();

      // Decode base64 RGBA data
      const binary = atob(data);
      const bytes = new Uint8ClampedArray(binary.length);
      for (let i = 0; i < binary.length; i++) {
        bytes[i] = binary.charCodeAt(i);
      }

      // Resize canvas if needed
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width;
        canvas.height = height;
      }

      const ctx = canvas.getContext("2d");
      if (!ctx) return;

      const imageData = new ImageData(bytes, width, height);
      ctx.putImageData(imageData, 0, 0);

      onStats?.({ minVal: min_val, maxVal: max_val, minPos: min_pos, maxPos: max_pos, width, height });
    });

    resetWatchdog();
  }, [canvasRef, onStats, resetWatchdog]);

  const stop = useCallback(() => {
    unlistenRef.current?.();
    unlistenRef.current = null;
    if (watchdogRef.current) {
      clearTimeout(watchdogRef.current);
      watchdogRef.current = null;
    }
  }, []);

  useEffect(() => {
    return () => stop();
  }, [stop]);

  return { start, stop };
}
