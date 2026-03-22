import { useState, useEffect } from "react";

interface AgcControlsProps {
  getAgcEnable: () => Promise<boolean>;
  setAgcEnable: (enable: boolean) => Promise<void>;
  getAgcPolicy: () => Promise<number>;
  setAgcPolicy: (policy: number) => Promise<void>;
}

export function AgcControls({
  getAgcEnable,
  setAgcEnable,
  getAgcPolicy,
  setAgcPolicy,
}: AgcControlsProps) {
  const [enabled, setEnabled] = useState(false);
  const [policy, setPolicy] = useState(0);

  useEffect(() => {
    getAgcEnable().then(setEnabled).catch(() => {});
    getAgcPolicy().then(setPolicy).catch(() => {});
  }, [getAgcEnable, getAgcPolicy]);

  const handleEnableToggle = async () => {
    const newVal = !enabled;
    await setAgcEnable(newVal);
    setEnabled(newVal);
  };

  const handlePolicyChange = async (newPolicy: number) => {
    await setAgcPolicy(newPolicy);
    setPolicy(newPolicy);
  };

  return (
    <div className="control-section">
      <h3>AGC</h3>
      <label className="toggle">
        <input
          type="checkbox"
          checked={enabled}
          onChange={handleEnableToggle}
        />
        Enable AGC
      </label>
      {enabled && (
        <label>
          Mode
          <select
            value={policy}
            onChange={(e) => handlePolicyChange(Number(e.target.value))}
          >
            <option value={0}>Linear</option>
            <option value={1}>HEQ (Histogram Equalization)</option>
          </select>
        </label>
      )}
    </div>
  );
}
