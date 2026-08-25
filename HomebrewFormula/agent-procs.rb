class AgentProcs < Formula
  desc "Concurrent process runner for AI agents"
  homepage "https://github.com/jkhoffman/agent-procs"
  url "https://github.com/jkhoffman/agent-procs/archive/6faae0fae78c6255fe8724cf13c70f74993fb524.tar.gz"
  version "0.6.2"
  sha256 "64fa7afc14264fd077a73e4bd747c72eb7a38626bbc2b0ec600d06324e6f72e8"
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
