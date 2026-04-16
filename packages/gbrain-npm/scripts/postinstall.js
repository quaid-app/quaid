const crypto = require("crypto");
const fs = require("fs");
const https = require("https");
const path = require("path");

const packageJson = require("../package.json");

const version = packageJson.version;
const tag = `v${version}`;
const releaseTagUrl = `https://github.com/macro88/gigabrain/releases/tag/${tag}`;

function platformToAsset() {
  if (process.platform === "darwin" && process.arch === "arm64") {
    return "gbrain-darwin-arm64";
  }
  if (process.platform === "darwin" && process.arch === "x64") {
    return "gbrain-darwin-x86_64";
  }
  if (process.platform === "linux" && process.arch === "x64") {
    return "gbrain-linux-x86_64";
  }
  if (process.platform === "linux" && process.arch === "arm64") {
    return "gbrain-linux-aarch64";
  }
  return null;
}

function manualInstallMessage(reason) {
  console.warn(`[gbrain] ${reason}`);
  console.warn(`[gbrain] Install manually from ${releaseTagUrl}`);
}

function gracefulSkip(reason) {
  manualInstallMessage(reason);
  process.exitCode = 0;
}

function request(url, redirectCount = 0) {
  return new Promise((resolve, reject) => {
    const timeoutMs = 60_000;
    const req = https.get(
      url,
      {
        headers: {
          "user-agent": "gbrain-npm-postinstall"
        },
        timeout: timeoutMs
      },
      (res) => {
        const status = res.statusCode || 0;

        if ([301, 302, 303, 307, 308].includes(status)) {
          const location = res.headers.location;
          res.resume();
          if (!location) {
            reject(new Error(`Redirect without location for ${url}`));
            return;
          }
          if (redirectCount >= 5) {
            reject(new Error(`Too many redirects fetching ${url}`));
            return;
          }
          resolve(request(location, redirectCount + 1));
          return;
        }

        if (status < 200 || status >= 300) {
          res.resume();
          reject(new Error(`HTTP ${status} fetching ${url}`));
          return;
        }

        res.setTimeout(timeoutMs, () => {
          res.destroy(new Error(`Socket timeout after ${timeoutMs / 1000}s reading ${url}`));
        });

        resolve(res);
      }
    );

    req.on("timeout", () => {
      req.destroy(new Error(`Connection timeout after ${timeoutMs / 1000}s for ${url}`));
    });
    req.on("error", reject);
  });
}

async function downloadText(url) {
  const res = await request(url);
  return new Promise((resolve, reject) => {
    let body = "";
    res.setEncoding("utf8");
    res.on("data", (chunk) => {
      body += chunk;
    });
    res.on("end", () => resolve(body));
    res.on("error", reject);
  });
}

async function downloadFile(url, destination) {
  const res = await request(url);
  await fs.promises.mkdir(path.dirname(destination), { recursive: true });

  await new Promise((resolve, reject) => {
    const file = fs.createWriteStream(destination);
    res.pipe(file);
    res.on("error", reject);
    file.on("finish", () => file.close(resolve));
    file.on("error", reject);
  });
}

async function sha256(filePath) {
  const hash = crypto.createHash("sha256");
  const stream = fs.createReadStream(filePath);

  await new Promise((resolve, reject) => {
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("end", resolve);
    stream.on("error", reject);
  });

  return hash.digest("hex");
}

function printDbTip() {
  console.log("");
  console.log("Tip: Set GBRAIN_DB in your shell profile to avoid passing --db on every command:");
  console.log("  echo 'export GBRAIN_DB=\"$HOME/brain.db\"' >> ~/.zshrc");
  console.log("  echo 'export GBRAIN_DB=\"$HOME/brain.db\"' >> ~/.bashrc");
}

async function main() {
  const assetName = platformToAsset();
  if (!assetName) {
    gracefulSkip(`Unsupported platform ${process.platform}/${process.arch}; skipping binary download.`);
    return;
  }

  const binaryUrl = `https://github.com/macro88/gigabrain/releases/download/${tag}/${assetName}`;
  const checksumUrl = `${binaryUrl}.sha256`;
  const binDir = path.join(__dirname, "..", "bin");
  const binaryPath = path.join(binDir, "gbrain.bin");
  const tempBinaryPath = path.join(binDir, "gbrain.download");

  try {
    const [checksumText] = await Promise.all([
      downloadText(checksumUrl),
      fs.promises.mkdir(binDir, { recursive: true })
    ]);

    await downloadFile(binaryUrl, tempBinaryPath);

    const expectedHash = checksumText.trim().split(/\s+/)[0];
    const actualHash = await sha256(tempBinaryPath);

    if (!expectedHash || actualHash !== expectedHash) {
      await fs.promises.rm(tempBinaryPath, { force: true });
      throw new Error(`SHA-256 mismatch for ${assetName}`);
    }

    await fs.promises.rename(tempBinaryPath, binaryPath);
    fs.chmodSync(binaryPath, 0o755);

    console.log(`[gbrain] Installed ${assetName} from GitHub Releases.`);
    printDbTip();
  } catch (error) {
    await fs.promises.rm(tempBinaryPath, { force: true }).catch(() => {});
    const message = error instanceof Error ? error.message : String(error);

    if (/Unsupported platform|HTTP \d+ fetching|ENOTFOUND|ECONNRESET|ECONNREFUSED|ETIMEDOUT|getaddrinfo|network|timeout/i.test(message)) {
      gracefulSkip(`Could not download the platform binary (${message}).`);
      return;
    }

    console.error(`[gbrain] ${message}`);
    process.exitCode = 1;
  }
}

main();
