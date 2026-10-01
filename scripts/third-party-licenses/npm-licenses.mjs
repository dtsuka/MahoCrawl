// フロントエンドにバンドルされ得る本番依存（dependencies とその推移的依存）の
// ライセンス本文を標準出力へ書き出す。devDependencies は配布物に含まれないため対象外。
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";

const LICENSE_FILE_PATTERN = /^(licen[cs]e|copying)/i;

/** npm ls から本番依存のディレクトリ一覧を取得する（先頭はプロジェクト自身なので除く） */
function listProductionPackageDirs(projectRoot) {
  // Windows の npm は npm.cmd のため、シェル経由で起動する（引数は固定値のみ）
  const output = execFileSync("npm", ["ls", "--omit=dev", "--all", "--parseable"], {
    cwd: projectRoot,
    encoding: "utf8",
    shell: process.platform === "win32",
  });
  const dirs = output.split(/\r?\n/).filter(Boolean).slice(1);
  return [...new Set(dirs)];
}

/** パッケージ1件分のライセンス情報を文字列にする */
function formatPackage(packageDir, projectRoot) {
  const manifest = JSON.parse(readFileSync(join(packageDir, "package.json"), "utf8"));
  const licenseFiles = readdirSync(packageDir).filter((name) => LICENSE_FILE_PATTERN.test(name)).sort();
  if (licenseFiles.length === 0) {
    throw new Error(`ライセンスファイルが見つかりません: ${relative(projectRoot, packageDir)}`);
  }

  const header = [
    "--------------------------------------------------------------------------------",
    `Package: ${manifest.name} ${manifest.version}`,
    `License: ${manifest.license ?? "UNKNOWN"}`,
  ];
  const bodies = licenseFiles.map((name) => readFileSync(join(packageDir, name), "utf8").trim());
  return [...header, "", ...bodies, ""].join("\n");
}

const projectRoot = process.argv[2] ?? process.cwd();
const packages = listProductionPackageDirs(projectRoot)
  .map((dir) => ({ dir, name: JSON.parse(readFileSync(join(dir, "package.json"), "utf8")).name }))
  .sort((a, b) => a.name.localeCompare(b.name));

process.stdout.write(packages.map(({ dir }) => formatPackage(dir, projectRoot)).join("\n"));
