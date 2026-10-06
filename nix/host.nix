# Telmo.app built with the Command Line Tools swiftc (same approach as the
# wifi-panel package: needs the macOS 26 SDK for NSGlassEffectView, no SwiftPM).
# Not signed here; sign at install time with a Developer ID.
{ stdenv, fetchFromGitHub }:

let
  swiftterm = fetchFromGitHub {
    owner = "migueldeicaza";
    repo = "SwiftTerm";
    rev = "v1.19.0";
    hash = "sha256-jsKRuBg+YVbp1zcZhPRvS+MWROpXKyvclTNgVhnDhh8=";
  };
in
stdenv.mkDerivation {
  pname = "telmo-host";
  version = "0.1.0";
  src = ../host/macos;
  dontConfigure = true;
  # swiftc comes from /Library/Developer/CommandLineTools, outside the store.
  __noChroot = true;
  # Remote builders may lack the Command Line Tools; this must build here.
  preferLocalBuild = true;
  allowSubstitutes = false;
  buildPhase = ''
    SWIFTTERM_SRC=${swiftterm} OUT=$PWD/out sh ./build.sh
  '';
  installPhase = ''
    mkdir -p $out/Applications
    cp -R out/Telmo.app $out/Applications/
  '';
  meta = {
    description = "Telmo popup host app for macOS";
    platforms = [ "aarch64-darwin" ];
  };
}
