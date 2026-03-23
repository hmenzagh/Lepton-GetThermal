import { useState, useEffect } from "react";

interface TemperatureDisplayProps {
  getSpotTemperature: () => Promise<number>;
  streaming: boolean;
}

export function TemperatureDisplay({
  getSpotTemperature,
  streaming,
}: TemperatureDisplayProps) {
  const [tempC, setTempC] = useState<number | null>(null);
  const [useFahrenheit, setUseFahrenheit] = useState(false);

  useEffect(() => {
    if (!streaming) return;
    const interval = setInterval(async () => {
      try {
        const celsius = await getSpotTemperature();
        setTempC(celsius);
      } catch {
        /* Camera may not support radiometry */
      }
    }, 1000);
    return () => clearInterval(interval);
  }, [streaming, getSpotTemperature]);

  if (tempC === null) return null;

  const displayTemp = useFahrenheit ? tempC * 1.8 + 32 : tempC;
  const unit = useFahrenheit ? "\u00b0F" : "\u00b0C";

  return (
    <div
      className="temperature-display"
      onClick={() => setUseFahrenheit((f) => !f)}
      title="Click to toggle \u00b0C/\u00b0F"
    >
      <div className="temp-label">Spot</div>
      <span className="temp-value">{displayTemp.toFixed(1)}</span>
      <span className="temp-unit">{unit}</span>
    </div>
  );
}
