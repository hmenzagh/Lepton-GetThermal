import { useState, useCallback } from "react";

interface SpotmeterOverlayProps {
  canvasWidth: number;
  canvasHeight: number;
  onRoiChange: (
    rowStart: number,
    colStart: number,
    rowEnd: number,
    colEnd: number
  ) => void;
}

export function SpotmeterOverlay({
  canvasWidth,
  canvasHeight,
  onRoiChange,
}: SpotmeterOverlayProps) {
  const [roi, setRoi] = useState({ x: 0.5, y: 0.5 });

  const handleClick = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      const rect = e.currentTarget.getBoundingClientRect();
      const x = (e.clientX - rect.left) / rect.width;
      const y = (e.clientY - rect.top) / rect.height;
      setRoi({ x, y });

      const col = Math.round(x * canvasWidth);
      const row = Math.round(y * canvasHeight);
      const size = 2;
      onRoiChange(
        Math.max(0, row - size),
        Math.max(0, col - size),
        Math.min(canvasHeight - 1, row + size),
        Math.min(canvasWidth - 1, col + size)
      );
    },
    [canvasWidth, canvasHeight, onRoiChange]
  );

  return (
    <div
      className="spotmeter-overlay"
      onClick={handleClick}
      style={{
        position: "absolute",
        top: 0,
        left: 0,
        width: "100%",
        height: "100%",
        cursor: "crosshair",
        zIndex: 5,
      }}
    >
      {/* Crosshair reticle */}
      <div
        style={{
          position: "absolute",
          left: `${roi.x * 100}%`,
          top: `${roi.y * 100}%`,
          transform: "translate(-50%, -50%)",
          pointerEvents: "none",
        }}
      >
        {/* Outer ring */}
        <div
          style={{
            width: 24,
            height: 24,
            border: "1.5px solid rgba(0, 230, 118, 0.8)",
            borderRadius: "50%",
            position: "relative",
          }}
        >
          {/* Center dot */}
          <div
            style={{
              position: "absolute",
              left: "50%",
              top: "50%",
              width: 3,
              height: 3,
              background: "#00e676",
              borderRadius: "50%",
              transform: "translate(-50%, -50%)",
              boxShadow: "0 0 4px rgba(0, 230, 118, 0.6)",
            }}
          />
        </div>
        {/* Crosshair lines */}
        <div style={{
          position: "absolute", top: "50%", left: -6, width: 6, height: 1,
          background: "rgba(0, 230, 118, 0.5)", transform: "translateY(-50%)",
        }} />
        <div style={{
          position: "absolute", top: "50%", right: -6, width: 6, height: 1,
          background: "rgba(0, 230, 118, 0.5)", transform: "translateY(-50%)",
        }} />
        <div style={{
          position: "absolute", left: "50%", top: -6, width: 1, height: 6,
          background: "rgba(0, 230, 118, 0.5)", transform: "translateX(-50%)",
        }} />
        <div style={{
          position: "absolute", left: "50%", bottom: -6, width: 1, height: 6,
          background: "rgba(0, 230, 118, 0.5)", transform: "translateX(-50%)",
        }} />
      </div>
    </div>
  );
}
