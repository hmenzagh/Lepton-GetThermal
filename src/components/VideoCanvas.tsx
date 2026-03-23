import { useRef, useEffect } from "react";
import { useFrameStream, FrameStats } from "../hooks/useFrameStream";

interface VideoCanvasProps {
  streaming: boolean;
  onStats?: (stats: FrameStats) => void;
  onDisconnect?: () => void;
  className?: string;
}

export function VideoCanvas({ streaming, onStats, onDisconnect, className }: VideoCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { start, stop } = useFrameStream(canvasRef, onStats, onDisconnect);

  useEffect(() => {
    if (streaming) {
      start();
    } else {
      stop();
    }
  }, [streaming, start, stop]);

  return (
    <canvas
      ref={canvasRef}
      className={className}
      style={{
        imageRendering: "pixelated",
        width: "100%",
        height: "100%",
        objectFit: "contain",
        background: "#000",
        pointerEvents: streaming ? "auto" : "none",
      }}
    />
  );
}
