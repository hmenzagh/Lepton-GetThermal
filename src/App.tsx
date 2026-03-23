import { useState, useCallback } from "react";
import { VideoCanvas } from "./components/VideoCanvas";
import { ControlPanel } from "./components/ControlPanel";
import { TemperatureDisplay } from "./components/TemperatureDisplay";
import { PaletteBar } from "./components/PaletteBar";
import { SpotmeterOverlay } from "./components/SpotmeterOverlay";
import { MarkersOverlay } from "./components/MarkersOverlay";
import { useCamera } from "./hooks/useCamera";
import { FrameStats } from "./hooks/useFrameStream";
import { Palette, DEFAULT_WIDTH, DEFAULT_HEIGHT } from "./lib/types";
import "./App.css";

function App() {
  const camera = useCamera();
  const [palette, setPalette] = useState<Palette>("ironblack");
  const [frameStats, setFrameStats] = useState<FrameStats>({ minVal: 0, maxVal: 0, minPos: 0, maxPos: 0 });
  const [showMarkers, setShowMarkers] = useState(false);

  const handleStats = useCallback((stats: FrameStats) => {
    setFrameStats(stats);
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
    await camera.startStream();
  }, [camera.connect, camera.startStream]);

  const handlePolarityChange = useCallback(
    (polarity: number) => {
      camera.setPolarity(polarity);
    },
    [camera.setPolarity]
  );

  const handleIsothermChange = useCallback(
    (tempC: number | null) => {
      if (tempC === null) {
        camera.setIsotherm(0);
      } else {
        // Convert Celsius to raw Y16 (centikelvins: (C + 273.15) * 100)
        const raw = Math.round((tempC + 273.15) * 100);
        camera.setIsotherm(raw);
      }
    },
    [camera.setIsotherm]
  );

  const handleRoiChange = useCallback(
    (r1: number, c1: number, r2: number, c2: number) => {
      camera.setSpotmeterRoi(r1, c1, r2, c2);
    },
    [camera.setSpotmeterRoi]
  );

  const isStreaming = camera.state === "streaming";
  const isConnecting = camera.state === "connecting";
  const showRadiometry = camera.deviceInfo?.supports_radiometry ?? false;

  return (
    <div className="app">
      <ControlPanel
        deviceInfo={camera.deviceInfo}
        currentPalette={palette}
        onPaletteChange={handlePaletteChange}
        onFfc={camera.performFfc}
        onPolarityChange={handlePolarityChange}
        onIsothermChange={handleIsothermChange}
        showMarkers={showMarkers}
        onToggleMarkers={() => setShowMarkers((v) => !v)}
      />
      <main className="video-area">
        {(camera.state === "disconnected" || isConnecting) && (
          <button
            className={`connect-button ${isConnecting ? "connecting" : ""}`}
            onClick={handleConnect}
            disabled={isConnecting}
          >
            <div className="connect-icon">
              <svg viewBox="0 0 24 24">
                <circle cx="12" cy="12" r="3" />
                <path d="M12 1v4M12 19v4M4.22 4.22l2.83 2.83M16.95 16.95l2.83 2.83M1 12h4M19 12h4M4.22 19.78l2.83-2.83M16.95 7.05l2.83-2.83" />
              </svg>
            </div>
            <span className="connect-label">
              {isConnecting ? "Connecting..." : "Connect"}
            </span>
          </button>
        )}
        {camera.error && <div className="error" onClick={() => camera.clearError()} style={{ cursor: "pointer" }}>{camera.error}</div>}
        <div className="video-container">
          <VideoCanvas
            streaming={isStreaming}
            onStats={handleStats}
            className="thermal-video"
          />
          {isStreaming && showMarkers && (
            <MarkersOverlay
              width={DEFAULT_WIDTH}
              height={DEFAULT_HEIGHT}
              minPos={frameStats.minPos}
              maxPos={frameStats.maxPos}
              minVal={frameStats.minVal}
              maxVal={frameStats.maxVal}
            />
          )}
          {isStreaming && showRadiometry && (
            <SpotmeterOverlay
              canvasWidth={DEFAULT_WIDTH}
              canvasHeight={DEFAULT_HEIGHT}
              onRoiChange={handleRoiChange}
            />
          )}
        </div>
        {isStreaming && (
          <div className="status-bar">
            <div className="status-indicator">
              <div className="status-dot live" />
              <span>LIVE</span>
            </div>
            <span>{DEFAULT_WIDTH}x{DEFAULT_HEIGHT}</span>
          </div>
        )}
      </main>
      {isStreaming && (
        <aside className="info-panel">
          {showRadiometry && (
            <TemperatureDisplay
              getSpotTemperature={camera.getSpotTemperature}
              streaming={isStreaming}
            />
          )}
          <PaletteBar
            minVal={frameStats.minVal}
            maxVal={frameStats.maxVal}
            palette={palette}
          />
        </aside>
      )}
    </div>
  );
}

export default App;
