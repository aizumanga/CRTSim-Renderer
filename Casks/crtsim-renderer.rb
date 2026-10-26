cask "crtsim-renderer" do
  version "1.8.3"
  sha256 "cf887e65b469c036e76ac3447bc23f24831f691e38e74cc33d661b3691af63da"

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
