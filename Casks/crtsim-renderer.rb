cask "crtsim-renderer" do
  version "1.1.0"
  sha256 "18b81aa0b334200328e68e2812329ab5a7e6230e202c699c5e25435d160da31e"

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
