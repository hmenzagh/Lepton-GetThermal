import { VideoControls } from "./VideoControls";
import { AgcControls } from "./AgcControls";
import { DeviceInfo } from "./DeviceInfo";
import { Palette, DeviceInfo as DeviceInfoType } from "../lib/types";

interface ControlPanelProps {
  deviceInfo: DeviceInfoType | null;
  currentPalette: Palette;
  onPaletteChange: (palette: Palette) => void;
  onFfc: () => void;
  onPolarityChange: (polarity: number) => void;
  getAgcEnable: () => Promise<boolean>;
  setAgcEnable: (enable: boolean) => Promise<void>;
  getAgcPolicy: () => Promise<number>;
  setAgcPolicy: (policy: number) => Promise<void>;
}

export function ControlPanel({
  deviceInfo,
  currentPalette,
  onPaletteChange,
  onFfc,
  onPolarityChange,
  getAgcEnable,
  setAgcEnable,
  getAgcPolicy,
  setAgcPolicy,
}: ControlPanelProps) {
  return (
    <aside className="control-panel">
      <h2>Thermal V2</h2>
      <DeviceInfo info={deviceInfo} />
      <VideoControls
        currentPalette={currentPalette}
        onPaletteChange={onPaletteChange}
        onFfc={onFfc}
        onPolarityChange={onPolarityChange}
      />
      <AgcControls
        getAgcEnable={getAgcEnable}
        setAgcEnable={setAgcEnable}
        getAgcPolicy={getAgcPolicy}
        setAgcPolicy={setAgcPolicy}
      />
    </aside>
  );
}
