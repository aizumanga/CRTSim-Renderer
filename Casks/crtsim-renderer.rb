cask "crtsim-renderer" do
  version "1.5.0"
  sha256 "8c280584f4dcc3ebbf21a767442d14b1e8acc9e77aa425b75c0a3df28d2877cc"

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
