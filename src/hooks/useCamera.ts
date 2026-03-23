import { useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ConnectionState, DeviceInfo, Palette } from "../lib/types";

export function useCamera() {
  const [state, setState] = useState<ConnectionState>("disconnected");
  const [deviceInfo, setDeviceInfo] = useState<DeviceInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const connect = useCallback(async () => {
    try {
      setState("connecting");
      setError(null);
      await invoke("connect_camera");
      const info = await invoke<DeviceInfo>("get_device_info");
      setDeviceInfo(info);
      setState("connected");
    } catch (e) {
      setError(String(e));
      setState("error");
    }
  }, []);

  const startStream = useCallback(async () => {
    try {
      await invoke("start_stream");
      setState("streaming");
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const stopStream = useCallback(async () => {
    try {
      await invoke("stop_stream");
      setState("connected");
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const setPalette = useCallback(async (palette: Palette) => {
    try {
      await invoke("set_palette", { palette });
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const performFfc = useCallback(async () => {
    try {
      await invoke("perform_ffc");
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const setPolarity = useCallback(async (polarity: number) => {
    await invoke("set_polarity", { polarity });
  }, []);

  const setIsotherm = useCallback(async (rawThreshold: number) => {
    await invoke("set_isotherm", { rawThreshold });
  }, []);

  const clearError = useCallback(() => {
    setError(null);
    if (state === "error") setState("disconnected");
  }, [state]);

  const setGainMode = useCallback(async (mode: number) => {
    await invoke("set_gain_mode", { mode });
  }, []);

  const getSpotTemperature = useCallback(async (): Promise<number> => {
    return invoke<number>("get_spot_temperature");
  }, []);

  const setSpotmeterRoi = useCallback(
    async (rowStart: number, colStart: number, rowEnd: number, colEnd: number) => {
      try {
        await invoke("set_spotmeter_roi", {
          rowStart: Math.round(rowStart),
          colStart: Math.round(colStart),
          rowEnd: Math.round(rowEnd),
          colEnd: Math.round(colEnd),
        });
      } catch (e) {
        setError(`ROI error: ${String(e)}`);
      }
    },
    []
  );

  return {
    state,
    deviceInfo,
    error,
    connect,
    startStream,
    stopStream,
    setPalette,
    performFfc,
    setPolarity,
    setIsotherm,
    clearError,
    setGainMode,
    getSpotTemperature,
    setSpotmeterRoi,
  };
}
