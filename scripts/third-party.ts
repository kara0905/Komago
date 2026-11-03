// Régénère THIRD-PARTY.md : bibliothèques embarquées dans l'application et leurs
// licences. À relancer après chaque changement de dépendances :
//   bun run third-party
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { spawnSync } from "node:child_process";

type Row = { name: string; version: string; license: string };

// Toutes les cibles : un paquet Windows ou Linux embarque d'autres crates que macOS.
const tree = spawnSync(
  "cargo",
  ["tree", "-e", "normal", "--target", "all", "--prefix", "none", "--format", "{p}|{l}"],
  { cwd: "src-tauri", encoding: "utf8" },
);
if (tree.status !== 0) throw new Error(tree.stderr);

const crates = new Map<string, Row>();
for (const line of tree.stdout.split("\n")) {
  const [spec, license = ""] = line.replace(/ \((\*|proc-macro)\)/g, "").split("|");
  const match = spec?.match(/^(\S+) v(\S+)/);
  if (!match || spec.includes("(/")) continue; // Komago lui-même
  crates.set(`${match[1]}@${match[2]}`, { name: match[1], version: match[2], license: license.trim() || "voir le dépôt" });
}

// Côté interface, seules les dépendances d'exécution finissent dans l'application.
const npm = new Map<string, Row>();
function walk(name: string) {
  const file = `node_modules/${name}/package.json`;
  if (npm.has(name) || !existsSync(file)) return;
  const pkg = JSON.parse(readFileSync(file, "utf8"));
  npm.set(name, { name, version: pkg.version, license: pkg.license ?? "voir le dépôt" });
  for (const dependency of Object.keys(pkg.dependencies ?? {})) walk(dependency);
}
for (const name of Object.keys(JSON.parse(readFileSync("package.json", "utf8")).dependencies)) walk(name);

const table = (rows: Row[]) =>
  ["| Bibliothèque | Version | Licence |", "| --- | --- | --- |"]
    .concat(rows.sort((a, b) => a.name.localeCompare(b.name)).map((r) => `| ${r.name} | ${r.version} | ${r.license} |`))
    .join("\n");

writeFileSync(
  "THIRD-PARTY.md",
  `# Composants tiers

Komago s'appuie sur les bibliothèques ci-dessous, distribuées sous leurs propres
licences. Leurs textes complets accompagnent leurs sources, sur
[crates.io](https://crates.io) et [npmjs.com](https://www.npmjs.com).

Fichier généré par \`bun run third-party\` : ne pas le modifier à la main.

## Licences à conditions particulières

- **UnRAR** (lecture des CBR, via \`unrar_sys\`) : code source gratuit de RARLAB,
  utilisable pour lire des archives RAR mais pas pour créer un compresseur RAR.
  Texte complet : [licenses/UnRAR.txt](licenses/UnRAR.txt). Cette licence est
  incompatible avec la GPL.
- **MPL-2.0** (\`cssparser\`, \`selectors\`, \`dtoa-short\`, \`option-ext\`) : copyleft
  limité aux fichiers de ces bibliothèques, utilisées sans modification.
- **CDLA-Permissive-2.0** (\`webpki-root-certs\`) : liste des certificats racines.

## Rust (${crates.size} paquets)

${table([...crates.values()])}

## Interface (${npm.size} paquets)

${table([...npm.values()])}
`,
);
console.log(`THIRD-PARTY.md : ${crates.size} crates, ${npm.size} paquets npm`);
