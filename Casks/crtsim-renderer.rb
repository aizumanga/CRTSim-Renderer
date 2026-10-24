cask "crtsim-renderer" do
  version "1.8.0"
  sha256 "e9a3b8bff1b947d9d47a99b2671d71d30d70b01d6ffab2f05195d4c3b7f3dbf7"

  url "https://github.com/aizumanga/CRTSim-Renderer/releases/download/v#{version}/CRTSim-Renderer-v#{version}-macos-arm64.tar.gz"
  name "CRTSim Renderer"
  desc "Renders images and video through CRTSim, a simulated CRT television"
  homepage "https://github.com/aizumanga/CRTSim-Renderer"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on arch: :arm64

  app "CRTSim-Renderer-v#{version}-macos-arm64/CRTSim Renderer.app"
  binary "CRTSim-Renderer-v#{version}-macos-arm64/crtsim"

  zap trash: "~/Library/Application Support/CRTSim-Renderer"

  caveats <<~EOS
    CRTSim Renderer is ad-hoc signed, without an Apple Developer ID. If macOS will not open it,
    allow it in System Settings, Privacy & Security.

    Video needs FFmpeg and ffprobe:
      brew install ffmpeg
  EOS
end
