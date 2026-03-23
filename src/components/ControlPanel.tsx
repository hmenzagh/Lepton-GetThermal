import { VideoControls } from "./VideoControls";
import { DeviceInfo } from "./DeviceInfo";
import { Palette, DeviceInfo as DeviceInfoType } from "../lib/types";

interface ControlPanelProps {
  deviceInfo: DeviceInfoType | null;
  currentPalette: Palette;
  onPaletteChange: (palette: Palette) => void;
  onFfc: () => void;
  onPolarityChange: (polarity: number) => void;
  onIsothermChange: (tempC: number | null) => void;
  showMarkers: boolean;
  onToggleMarkers: () => void;
}

export function ControlPanel({
  deviceInfo,
  currentPalette,
  onPaletteChange,
  onFfc,
  onPolarityChange,
  onIsothermChange,
  showMarkers,
  onToggleMarkers,
}: ControlPanelProps) {
  return (
    <aside className="control-panel">
      <div className="panel-header">
        <h2>Thermal</h2>
        <span className="version-badge">V2</span>
      </div>
      <DeviceInfo info={deviceInfo} />
      <VideoControls
        currentPalette={currentPalette}
        onPaletteChange={onPaletteChange}
        onFfc={onFfc}
        onPolarityChange={onPolarityChange}
        onIsothermChange={onIsothermChange}
        showMarkers={showMarkers}
        onToggleMarkers={onToggleMarkers}
      />
    </aside>
  );
}
