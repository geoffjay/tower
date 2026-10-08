# typed: false
# frozen_string_literal: true

# Homebrew formula for tower.
#
# This file is the source of truth for the formula published to the
# geoffjay/homebrew-tap tap. It is regenerated for each release by
# scripts/gen-homebrew-formula.sh, which fills in the version and the
# per-target sha256 checksums from the release's checksums.txt.
#
# See docs/knowledgebase/concepts/releases.md for the release workflow.
class Tower < Formula
  desc "Control and visibility for herds of coding agents"
  homepage "https://github.com/geoffjay/tower"
  version "0.0.0"
  license "MIT OR Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/geoffjay/tower/releases/download/v#{version}/tower-aarch64-apple-darwin.tar.gz"
      sha256 "SHA256_AARCH64_APPLE_DARWIN"
    end
    on_intel do
      url "https://github.com/geoffjay/tower/releases/download/v#{version}/tower-x86_64-apple-darwin.tar.gz"
      sha256 "SHA256_X86_64_APPLE_DARWIN"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/geoffjay/tower/releases/download/v#{version}/tower-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "SHA256_AARCH64_UNKNOWN_LINUX_GNU"
    end
    on_intel do
      url "https://github.com/geoffjay/tower/releases/download/v#{version}/tower-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "SHA256_X86_64_UNKNOWN_LINUX_GNU"
    end
  end

  def install
    bin.install "tower"
  end

  test do
    assert_match "tower", shell_output("#{bin}/tower --version")
  end
end
