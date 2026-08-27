class AgentProcs < Formula
  desc "Concurrent process runner for AI agents"
  homepage "https://github.com/jkhoffman/agent-procs"
  url "https://github.com/jkhoffman/agent-procs/archive/808e125d1dd9baf3087f13eec71785f63de26c49.tar.gz"
  version "0.6.3"
  sha256 "6f52b5b04b427822fd5e3b2742d7bb15125e40ef16685519310dfb5489929853"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_equal "agent-procs #{version}\n", shell_output("#{bin}/agent-procs --version")
    assert_match "_agent-procs() {", shell_output("#{bin}/agent-procs completions bash")
  end
end
