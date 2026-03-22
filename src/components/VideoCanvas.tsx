import { useRef, useEffect } from "react";
import { useFrameStream } from "../hooks/useFrameStream";

interface VideoCanvasProps {
  streaming: boolean;
  onStats?: (minVal: number, maxVal: number) => void;
  className?: string;
}

export function VideoCanvas({ streaming, onStats, className }: VideoCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { start, stop } = useFrameStream(canvasRef, onStats);

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
      }}
    />
  );
}
