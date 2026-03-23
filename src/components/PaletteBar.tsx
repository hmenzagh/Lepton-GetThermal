interface PaletteBarProps {
  minVal: number;
  maxVal: number;
  palette: string;
}

// Sampled from the actual Rust LUT data at key indices to match the real colorization
const gradientMap: Record<string, string> = {
  ironblack: `linear-gradient(to top,
    rgb(255,255,255) 0%,
    rgb(129,129,129) 25%,
    rgb(0,0,0) 50%,
    rgb(31,0,118) 56%,
    rgb(207,34,95) 70%,
    rgb(243,141,13) 82%,
    rgb(255,224,30) 93%,
    rgb(255,255,24) 100%
  )`,
  rainbow: `linear-gradient(to top,
    rgb(1,3,74) 0%,
    rgb(0,94,208) 25%,
    rgb(67,171,60) 50%,
    rgb(255,131,31) 75%,
    rgb(255,220,196) 100%
  )`,
  grayscale: "linear-gradient(to top, rgb(0,0,0), rgb(255,255,255))",
};

/** Convert raw Y16 value to Celsius.
 *  Values > 10000 are in centikelvins (0.01K), otherwise decikelvins (0.1K). */
function rawToCelsius(raw: number): string {
  if (raw > 10000) {
    return ((raw - 27315) / 100).toFixed(1);
  }
  return ((raw - 2731.5) / 10).toFixed(1);
}

export function PaletteBar({ minVal, maxVal, palette }: PaletteBarProps) {
  const midVal = Math.round((minVal + maxVal) / 2);

  return (
    <div className="palette-bar">
      <div
        className="palette-gradient"
        style={{
          background: gradientMap[palette] || gradientMap.ironblack,
        }}
      />
      <div className="palette-labels">
        <span>{rawToCelsius(maxVal)}&deg;</span>
        <span>{rawToCelsius(midVal)}&deg;</span>
        <span>{rawToCelsius(minVal)}&deg;</span>
      </div>
    </div>
  );
}
