/**
 * Package-local files of generated Typert artifacts. The tsdown plugin and
 * standalone workspace generation both write through this module, so a pass
 * over the same artifacts leaves the same bytes on disk.
 * @module @deepseek-ai/dsh-typert-generator/artifact-files
 */
import type { WorkspaceEmitResult } from './workspace.ts';
/**
 * Write one package's artifacts under `<packageDir>/lib`: `typert.<face>.js`
 * and `typert.<face>.d.ts` per artifact, plus the three
 * `typert.remote-client.*` files when an artifact carries Remote methods. A
 * Host artifact without Remote methods removes any Remote client files left
 * by an earlier pass. Files are written one by one, so a failed write can
 * leave this package's outputs partially replaced.
 * @param packageDir - absolute package root.
 * @param artifacts - artifacts of that package only.
 * @throws A file-system error when the output directory or a file cannot be written.
 */
export declare function writePackageArtifacts(packageDir: string, artifacts: readonly WorkspaceEmitResult[]): void;
/**
 * Write every artifact of one workspace pass into its own package, in
 * artifact order, with {@link writePackageArtifacts}. A failed write stops
 * the pass; earlier packages keep their new files and later ones keep their
 * previous files.
 * @param root - workspace root that each artifact's `packageRoot` is relative to.
 * @param artifacts - artifacts returned by one `WorkspaceTypertGenerator` pass.
 * @throws A file-system error when an output directory or file cannot be written.
 */
export declare function writeWorkspaceArtifacts(root: string, artifacts: readonly WorkspaceEmitResult[]): void;
//# sourceMappingURL=artifact-files.d.ts.map