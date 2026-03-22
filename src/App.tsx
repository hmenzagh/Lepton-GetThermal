import { useState, useCallback } from "react";
import { VideoCanvas } from "./components/VideoCanvas";
import { ControlPanel } from "./components/ControlPanel";
import { TemperatureDisplay } from "./components/TemperatureDisplay";
import { PaletteBar } from "./components/PaletteBar";
import { SpotmeterOverlay } from "./components/SpotmeterOverlay";
import { useCamera } from "./hooks/useCamera";
import { Palette, DEFAULT_WIDTH, DEFAULT_HEIGHT, DEFAULT_FPS } from "./lib/types";
import "./App.css";

function App() {
  const camera = useCamera();
  const [palette, setPalette] = useState<Palette>("ironblack");
  const [frameStats, setFrameStats] = useState({ min: 0, max: 0 });

  const handleStats = useCallback((min: number, max: number) => {
    setFrameStats({ min, max });
  }, []);

  const handlePaletteChange = useCallback(
    async (p: Palette) => {
      await camera.setPalette(p);
      setPalette(p);
    },
    [camera.setPalette]
  );

  const handleConnect = useCallback(async () => {
    await camera.connect();
    await camera.startStream(DEFAULT_WIDTH, DEFAULT_HEIGHT, DEFAULT_FPS);
  }, [camera.connect, camera.startStream]);

  const handlePolarityChange = useCallback(
    (polarity: number) => {
      camera.setPolarity(polarity);
    },
    [camera.setPolarity]
  );

  const handleRoiChange = useCallback(
    (r1: number, c1: number, r2: number, c2: number) => {
      camera.setSpotmeterRoi(r1, c1, r2, c2);
    },
    [camera.setSpotmeterRoi]
  );

  const isStreaming = camera.state === "streaming";
  const showRadiometry = camera.deviceInfo?.supports_radiometry ?? false;

  return (
    <div className="app">
      <ControlPanel
        deviceInfo={camera.deviceInfo}
        currentPalette={palette}
        onPaletteChange={handlePaletteChange}
        onFfc={camera.performFfc}
        onPolarityChange={handlePolarityChange}
        getAgcEnable={camera.getAgcEnable}
        setAgcEnable={camera.setAgcEnable}
        getAgcPolicy={camera.getAgcPolicy}
        setAgcPolicy={camera.setAgcPolicy}
      />
      <main className="video-area">
        {camera.state === "disconnected" && (
          <button className="connect-button" onClick={handleConnect}>
            Connect Camera
          </button>
        )}
        {camera.error && <div className="error">{camera.error}</div>}
        <div className="video-container">
          <VideoCanvas
            streaming={isStreaming}
            onStats={handleStats}
            className="thermal-video"
          />
          {isStreaming && showRadiometry && (
            <SpotmeterOverlay
              canvasWidth={DEFAULT_WIDTH}
              canvasHeight={DEFAULT_HEIGHT}
              onRoiChange={handleRoiChange}
            />
          )}
        </div>
      </main>
      {showRadiometry && (
        <aside className="info-panel">
          <TemperatureDisplay
            getSpotTemperature={camera.getSpotTemperature}
            streaming={isStreaming}
          />
          <PaletteBar
            minVal={frameStats.min}
            maxVal={frameStats.max}
            palette={palette}
          />
        </aside>
      )}
    </div>
  );
}

export default App;
