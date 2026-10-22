cask "crtsim-renderer" do
  version "1.6.0"
  sha256 "79d546fa91c132b0a795c9dbbeb7d39650317a8f232c437b1a7e44cc850f6ed9"

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
