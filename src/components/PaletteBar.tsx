interface PaletteBarProps {
  minVal: number;
  maxVal: number;
  palette: string;
}

export function PaletteBar({ minVal, maxVal, palette }: PaletteBarProps) {
  const midVal = Math.round((minVal + maxVal) / 2);

  const gradientMap: Record<string, string> = {
    ironblack:
      "linear-gradient(to top, #000, #2a0a00, #8b2500, #ff6600, #ffcc00, #fff)",
    rainbow: "linear-gradient(to top, #00f, #0ff, #0f0, #ff0, #f00)",
    grayscale: "linear-gradient(to top, #000, #fff)",
  };

  return (
    <div className="palette-bar">
      <div
        className="palette-gradient"
        style={{
          background: gradientMap[palette] || gradientMap.ironblack,
        }}
      />
      <div className="palette-labels">
        <span>{maxVal}</span>
        <span>{midVal}</span>
        <span>{minVal}</span>
      </div>
    </div>
  );
}
