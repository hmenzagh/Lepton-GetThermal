import { Palette } from "../lib/types";

interface VideoControlsProps {
  onPaletteChange: (palette: Palette) => void;
  onFfc: () => void;
  onPolarityChange: (polarity: number) => void;
  currentPalette: Palette;
}

export function VideoControls({
  onPaletteChange,
  onFfc,
  onPolarityChange,
  currentPalette,
}: VideoControlsProps) {
  return (
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
      <button onClick={onFfc} className="ffc-button">
        Run FFC
      </button>
    </div>
  );
}
