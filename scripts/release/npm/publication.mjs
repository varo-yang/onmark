// Release publication admits one complete version and makes retries idempotent.

const PACKAGE_ORDER = Object.freeze([
  "@onmark/cli-darwin-arm64",
  "@onmark/cli-linux-x64",
  "@onmark/cli-win32-x64",
  "@onmark/cli",
]);

export const PUBLICATION_PACKAGE_COUNT = PACKAGE_ORDER.length;

export function admitPublication(tag, archives) {
  if (
    !Array.isArray(archives) ||
    archives.length !== PUBLICATION_PACKAGE_COUNT
  ) {
    throw new Error(
      `release publication requires exactly ${PUBLICATION_PACKAGE_COUNT} package archives`,
    );
  }

  const byName = new Map();
  for (const archive of archives) {
    admitArchive(archive);
    if (byName.has(archive.name)) {
      throw new Error(`release publication contains duplicate ${archive.name}`);
    }
    byName.set(archive.name, archive);
  }

  const packages = PACKAGE_ORDER.map((name) => {
    const archive = byName.get(name);
    if (archive === undefined) {
      throw new Error(`release publication is missing ${name}`);
    }
    return archive;
  });
  const [first] = packages;
  const version = first.version;
  if (packages.some((archive) => archive.version !== version)) {
    throw new Error("release package archives do not share one version");
  }
  if (tag !== `v${version}`) {
    throw new Error(
      `release tag ${tag} does not match package version ${version}`,
    );
  }

  return Object.freeze({
    distributionTag: version.includes("-") ? "next" : "latest",
    packages: Object.freeze(packages),
    version,
  });
}

export async function publishAdmittedRelease(release, registry) {
  const outcomes = [];
  for (const archive of release.packages) {
    const outcome = await publishArchive(
      archive,
      release.distributionTag,
      registry,
    );
    outcomes.push(Object.freeze({ name: archive.name, outcome }));
  }
  return Object.freeze(outcomes);
}

function admitArchive(archive) {
  if (
    typeof archive !== "object" ||
    archive === null ||
    typeof archive.name !== "string" ||
    !PACKAGE_ORDER.includes(archive.name) ||
    typeof archive.version !== "string" ||
    archive.version.length === 0 ||
    typeof archive.integrity !== "string" ||
    !archive.integrity.startsWith("sha512-") ||
    typeof archive.path !== "string" ||
    archive.path.length === 0
  ) {
    throw new Error("release publication received invalid package metadata");
  }
}

async function publishArchive(archive, distributionTag, registry) {
  const existing = await registry.integrity(archive.name, archive.version);
  if (existing !== undefined) {
    requireSameIntegrity(archive, existing);
    return "reused";
  }

  try {
    await registry.publish(archive.path, distributionTag);
    return "published";
  } catch (publicationError) {
    return recoverPublicationRace(archive, registry, publicationError);
  }
}

async function recoverPublicationRace(archive, registry, publicationError) {
  let existing;
  try {
    existing = await registry.integrity(archive.name, archive.version);
  } catch (lookupError) {
    throw new AggregateError(
      [publicationError, lookupError],
      `publishing ${archive.name} failed and its registry state could not be verified`,
    );
  }
  if (existing === undefined) {
    throw publicationError;
  }
  requireSameIntegrity(archive, existing);
  return "reused";
}

function requireSameIntegrity(archive, existing) {
  if (existing !== archive.integrity) {
    throw new Error(
      `${archive.name}@${archive.version} already exists with different bytes`,
    );
  }
}
