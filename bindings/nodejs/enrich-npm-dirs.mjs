import { readdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const npmRoot = join(import.meta.dirname, 'npm');
const rootPackage = JSON.parse(readFileSync(join(import.meta.dirname, 'package.json'), 'utf8'));

for (const dir of readdirSync(npmRoot)) {
  // create-npm-dirs 生成的版本占位若未被改写,发布出去会是 0.0.0-stage——
  // 显式钉成主包版本,不依赖 napi prepublish 的内部改写
  const pkgPath = join(npmRoot, dir, 'package.json');
  if (existsSync(pkgPath)) {
    const pkg = JSON.parse(readFileSync(pkgPath, 'utf8'));
    if (pkg.version !== rootPackage.version) {
      pkg.version = rootPackage.version;
      writeFileSync(pkgPath, `${JSON.stringify(pkg, null, 2)}\n`);
    }
  }

  const readmePath = join(npmRoot, dir, 'README.md');
  if (!existsSync(readmePath)) continue;
  const readme = readFileSync(readmePath, 'utf8');
  const target = readme.match(/This is the \*\*(.+?)\*\* binary/)?.[1] ?? dir;
  writeFileSync(
    readmePath,
    `# \`@republicroad/zen-engine-${dir}\`

This is the **${target}** binary for [\`@republicroad/zen-engine\`](https://www.npmjs.com/package/@republicroad/zen-engine), the open-source [Node.js rules engine](https://gorules.io/open-source/javascript-rules-engine) from [GoRules](https://gorules.io).

- [Documentation](https://docs.gorules.io/developers/sdks/nodejs)
- [GitHub](https://github.com/gorules/zen)
`,
  );
}
