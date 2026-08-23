import { readFileSync, writeFileSync } from 'node:fs'

const version = (process.argv[2] ?? process.env.WOTSTAT_VERSION ?? '').trim()
const semver = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/

if (!semver.test(version)) {
  throw new Error(`Invalid release version: ${version || '<empty>'}`)
}

function updateJson(path) {
  const value = JSON.parse(readFileSync(path, 'utf8'))
  value.version = version
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`)
}

function replaceVersion(path, pattern) {
  const contents = readFileSync(path, 'utf8')
  if (!pattern.test(contents)) {
    throw new Error(`Cannot find package version in ${path}`)
  }
  writeFileSync(path, contents.replace(pattern, `$1${version}$2`))
}

updateJson('package.json')
updateJson('src-tauri/tauri.conf.json')
replaceVersion('src-tauri/Cargo.toml', /(\[package\][\s\S]*?\nversion\s*=\s*")[^"]+(")/)
replaceVersion(
  'src-tauri/Cargo.lock',
  /(\[\[package\]\]\r?\nname = "wotstat-repl"\r?\nversion = ")[^"]+(")/,
)

console.log(`Release version synchronized to ${version}`)
