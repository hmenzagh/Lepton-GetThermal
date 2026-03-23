import { useRef, useEffect, useState } from "react";

interface MarkersOverlayProps {
  width: number;
  height: number;
  minPos: number;
  maxPos: number;
  minVal: number;
  maxVal: number;
}

/** Convert raw Y16 to Celsius string */
function rawToC(raw: number): string {
  if (raw > 10000) {
    return ((raw - 27315) / 100).toFixed(1);
  }
  return ((raw - 2731.5) / 10).toFixed(1);
}

/**
 * Compute the actual rendered area of a canvas with object-fit: contain
 * within its parent container.
 */
function getContainedRect(container: HTMLElement, imgW: number, imgH: number) {
  const cw = container.clientWidth;
  const ch = container.clientHeight;
  const scale = Math.min(cw / imgW, ch / imgH);
  const w = imgW * scale;
  const h = imgH * scale;
  return {
    left: (cw - w) / 2,
    top: (ch - h) / 2,
    width: w,
    height: h,
  };
}

function Pin({ x, y, label, color, bgColor }: {
  x: number;
  y: number;
  label: string;
  color: string;
  bgColor: string;
}) {
  return (
    <div style={{
      position: "absolute",
      left: x,
      top: y,
      transform: "translate(-50%, -100%)",
      pointerEvents: "none",
      display: "flex",
      flexDirection: "column",
      alignItems: "center",
    }}>
      {/* Badge */}
      <div style={{
        background: bgColor,
        borderRadius: 4,
        padding: "2px 5px",
        display: "flex",
        alignItems: "center",
        gap: 4,
        boxShadow: "0 2px 6px rgba(0,0,0,0.5)",
        border: `1px solid ${color}`,
      }}>
        <span style={{
          fontFamily: "var(--font-mono)",
          fontSize: 10,
          fontWeight: 600,
          color: color,
          lineHeight: 1,
        }}>
          {label}
        </span>
      </div>
      {/* Stem */}
      <div style={{
        width: 1,
        height: 6,
        background: color,
        opacity: 0.7,
      }} />
      {/* Dot */}
      <div style={{
        width: 5,
        height: 5,
        borderRadius: "50%",
        background: color,
        boxShadow: `0 0 4px ${color}`,
      }} />
    </div>
  );
}

export function MarkersOverlay({ width, height, minPos, maxPos, minVal, maxVal }: MarkersOverlayProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [rect, setRect] = useState({ left: 0, top: 0, width: 1, height: 1 });

  useEffect(() => {
    const update = () => {
      const el = containerRef.current?.parentElement;
      if (el) setRect(getContainedRect(el, width, height));
    };
    update();
    const observer = new ResizeObserver(update);
    if (containerRef.current?.parentElement) {
      observer.observe(containerRef.current.parentElement);
    }
    return () => observer.disconnect();
  }, [width, height]);

  const minCol = minPos % width;
  const minRow = Math.floor(minPos / width);
  const maxCol = maxPos % width;
  const maxRow = Math.floor(maxPos / width);

  const pixelX = (col: number) => rect.left + (col / width) * rect.width;
  const pixelY = (row: number) => rect.top + (row / height) * rect.height;

  return (
    <div ref={containerRef} style={{ position: "absolute", inset: 0, pointerEvents: "none", zIndex: 4 }}>
      <Pin
        x={pixelX(minCol)}
        y={pixelY(minRow)}
        label={`${rawToC(minVal)}°`}
        color="#4fc3f7"
        bgColor="rgba(10, 20, 35, 0.85)"
      />
      <Pin
        x={pixelX(maxCol)}
        y={pixelY(maxRow)}
        label={`${rawToC(maxVal)}°`}
        color="#ff6e40"
        bgColor="rgba(35, 12, 5, 0.85)"
      />
    </div>
  );
}
