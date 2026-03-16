class AgentProcs < Formula
  desc "Concurrent process runner for AI agents"
  homepage "https://github.com/jkhoffman/agent-procs"
  url "https://github.com/jkhoffman/agent-procs/archive/refs/tags/v0.3.0.tar.gz"
  # sha256 "PLACEHOLDER" # Update with actual checksum after release
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "agent-procs", shell_output("#{bin}/agent-procs --version")
  end
end
