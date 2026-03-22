import { DeviceInfo as DeviceInfoType } from "../lib/types";

interface DeviceInfoProps {
  info: DeviceInfoType | null;
}

export function DeviceInfo({ info }: DeviceInfoProps) {
  if (!info)
    return <div className="control-section">No device connected</div>;

  return (
    <div className="control-section">
      <h3>Device</h3>
      <div className="info-row">
        <span>Part:</span>
        <span>{info.part_number || "\u2014"}</span>
      </div>
      <div className="info-row">
        <span>Serial:</span>
        <span>{info.serial_number || "\u2014"}</span>
      </div>
      <div className="info-row">
        <span>Radiometry:</span>
        <span>{info.supports_radiometry ? "Yes" : "No"}</span>
      </div>
    </div>
  );
}
