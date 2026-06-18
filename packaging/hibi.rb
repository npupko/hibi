# Homebrew formula (§12). Update `url`/`sha256` per release.
class Hibi < Formula
  desc "Deterministic CLI that keeps docs from silently going stale against the code they describe"
  homepage "https://github.com/npupko/hibi"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/npupko/hibi/releases/download/v0.1.0/hibi-darwin-arm64"
      sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
    on_intel do
      url "https://github.com/npupko/hibi/releases/download/v0.1.0/hibi-darwin-x64"
      sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/npupko/hibi/releases/download/v0.1.0/hibi-linux-arm64"
      sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
    on_intel do
      url "https://github.com/npupko/hibi/releases/download/v0.1.0/hibi-linux-x64"
      sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
  end

  def install
    bin.install Dir["*"].first => "hibi"
  end

  test do
    assert_match "hibi", shell_output("#{bin}/hibi version")
  end
end
