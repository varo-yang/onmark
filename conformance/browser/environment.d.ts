// Browser-conformance module declarations not owned by TypeScript itself.

declare module "*.css";

declare module "*.svg" {
  const source: string;
  export default source;
}

declare module "*.ttf" {
  const source: string;
  export default source;
}
