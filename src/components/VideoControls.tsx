import { useState } from "react";
import { Palette } from "../lib/types";

interface VideoControlsProps {
  onPaletteChange: (palette: Palette) => void;
  onFfc: () => void;
  onPolarityChange: (polarity: number) => void;
  onIsothermChange: (tempC: number | null) => void;
  onCapture: () => void;
  showMarkers: boolean;
  onToggleMarkers: () => void;
  upscaleEnabled: boolean;
  onToggleUpscale: () => void;
  currentPalette: Palette;
  streaming: boolean;
}

export function VideoControls({
  onPaletteChange,
  onFfc,
  onPolarityChange,
  onIsothermChange,
  onCapture,
  showMarkers,
  onToggleMarkers,
  upscaleEnabled,
  onToggleUpscale,
  currentPalette,
  streaming,
}: VideoControlsProps) {
  const [isothermEnabled, setIsothermEnabled] = useState(false);
  const [isothermTemp, setIsothermTemp] = useState(35);

  const handleIsothermToggle = () => {
    const next = !isothermEnabled;
    setIsothermEnabled(next);
    onIsothermChange(next ? isothermTemp : null);
  };

  const handleIsothermTemp = (val: number) => {
    setIsothermTemp(val);
    if (isothermEnabled) {
      onIsothermChange(val);
    }
  };

  return (
    <>
      <div className="control-section">
        <h3>Imaging</h3>
        <label>
          Palette
          <select
            value={currentPalette}
            onChange={(e) => onPaletteChange(e.target.value as Palette)}
          >
            <option value="ironblack">Iron Black</option>
            <option value="rainbow">Rainbow</option>
            <option value="grayscale">Grayscale</option>
          </select>
        </label>
        <label>
          Polarity
          <select onChange={(e) => onPolarityChange(Number(e.target.value))}>
            <option value={0}>White Hot</option>
            <option value={1}>Black Hot</option>
          </select>
        </label>
        <label className="toggle">
          <input
            type="checkbox"
            checked={showMarkers}
            onChange={onToggleMarkers}
          />
          Min/Max markers
        </label>
        <label className="toggle">
          <input
            type="checkbox"
            checked={upscaleEnabled}
            onChange={onToggleUpscale}
          />
          SR Upscale (3x)
        </label>
        <button onClick={onFfc} className="ffc-button">
          Run FFC
        </button>
        {streaming && (
          <button onClick={onCapture} className="ffc-button">
            Capture
          </button>
        )}
      </div>
      <div className="control-section">
        <h3>Isotherm</h3>
        <label className="toggle">
          <input
            type="checkbox"
            checked={isothermEnabled}
            onChange={handleIsothermToggle}
          />
          Enable
        </label>
        {isothermEnabled && (
          <label>
            Threshold
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <input
                type="range"
                min={-20}
                max={150}
                value={isothermTemp}
                onChange={(e) => handleIsothermTemp(Number(e.target.value))}
                style={{ flex: 1 }}
              />
              <span style={{ minWidth: 42, textAlign: "right", fontVariantNumeric: "tabular-nums" }}>
                {isothermTemp}&deg;C
              </span>
            </div>
          </label>
        )}
      </div>
    </>
  );
}
