# Homebrew formula (§12). Update `url`/`sha256` per release.
class ClaimEngine < Formula
  desc "Deterministic CLI that keeps docs from silently going stale against the code they describe"
  homepage "https://github.com/your-org/claim-engine"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/your-org/claim-engine/releases/download/v0.1.0/claim-engine-darwin-arm64"
      sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
    on_intel do
      url "https://github.com/your-org/claim-engine/releases/download/v0.1.0/claim-engine-darwin-x64"
      sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/your-org/claim-engine/releases/download/v0.1.0/claim-engine-linux-arm64"
      sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
    on_intel do
      url "https://github.com/your-org/claim-engine/releases/download/v0.1.0/claim-engine-linux-x64"
      sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
  end

  def install
    bin.install Dir["*"].first => "claim-engine"
  end

  test do
    assert_match "claim-engine", shell_output("#{bin}/claim-engine version")
  end
end
