import { Config } from "@remotion/cli/config";

Config.setVideoImageFormat("jpeg");
Config.setJpegQuality(95);
Config.setOverwriteOutput(true);
Config.setPixelFormat("yuv420p");
Config.setCodec("h264");
// Screen recordings stay crisp at this CRF and the file stays repo-friendly.
Config.setCrf(22);
Config.setConcurrency(3);
Config.setChromiumOpenGlRenderer("angle");
