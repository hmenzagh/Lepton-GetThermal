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
      console.log(`[spotmeter] click at x=${x.toFixed(2)}, y=${y.toFixed(2)}`);
      setRoi({ x, y });

      const col = Math.round(x * canvasWidth);
      const row = Math.round(y * canvasHeight);
      const size = 2;
      console.log(`[spotmeter] ROI: row=${row-size}-${row+size}, col=${col-size}-${col+size}`);
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
      <div
        className="spotmeter-marker"
        style={{
          position: "absolute",
          left: `${roi.x * 100}%`,
          top: `${roi.y * 100}%`,
          transform: "translate(-50%, -50%)",
          width: 20,
          height: 20,
          border: "2px solid #0f0",
          borderRadius: "50%",
          pointerEvents: "none",
        }}
      >
        <div
          style={{
            position: "absolute",
            left: "50%",
            top: "50%",
            width: 4,
            height: 4,
            background: "#0f0",
            borderRadius: "50%",
            transform: "translate(-50%, -50%)",
          }}
        />
      </div>
    </div>
  );
}
