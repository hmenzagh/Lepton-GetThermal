import { useRef, useEffect } from "react";
import { useFrameStream } from "../hooks/useFrameStream";

interface VideoCanvasProps {
  streaming: boolean;
  onStats?: (minVal: number, maxVal: number) => void;
  onCanvasClick?: (row: number, col: number) => void;
  className?: string;
}

export function VideoCanvas({ streaming, onStats, onCanvasClick, className }: VideoCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { start, stop } = useFrameStream(canvasRef, onStats);

  useEffect(() => {
    if (streaming) {
      start();
    } else {
      stop();
    }
  }, [streaming, start, stop]);

  const handleClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    console.log("[VideoCanvas] click detected, onCanvasClick=", !!onCanvasClick, "canvas=", !!canvasRef.current);
    if (!onCanvasClick || !canvasRef.current) return;
    const rect = canvasRef.current.getBoundingClientRect();
    // Map display coordinates to canvas pixel coordinates
    const scaleX = canvasRef.current.width / rect.width;
    const scaleY = canvasRef.current.height / rect.height;
    const col = Math.round((e.clientX - rect.left) * scaleX);
    const row = Math.round((e.clientY - rect.top) * scaleY);
    onCanvasClick(row, col);
  };

  return (
    <canvas
      ref={canvasRef}
      onClick={handleClick}
      className={className}
      style={{
        imageRendering: "pixelated",
        width: "100%",
        height: "100%",
        objectFit: "contain",
        background: "#000",
        cursor: onCanvasClick ? "crosshair" : "default",
      }}
    />
  );
}
