import { OfficialBrandMark, OfficialBrandName } from "./Brand.js";
/** Required service: the UI slot registry. */
export const inject = ['slots'];
/**
 * Fill the official sidebar brand slots. The former pixel-whale home frame
 * layer is retired with the whale mark (B1 品牌清底，方案 A）； the cloud
 * wordmark returns in the B2 visual batch.
 * @param ctx - Client root context.
 */
export function apply(ctx) {
    if (process.env.DSH_CLIENT_BUILD_PROFILE !== 'official')
        return;
    ctx.slots.inject('sidebar.brand.mark', () => ctx.slots.inject('sidebar.brand.name', function* () {
        yield ctx.slots.register({ name: 'sidebar.brand.mark' }, OfficialBrandMark);
        yield ctx.slots.register({ name: 'sidebar.brand.name' }, OfficialBrandName);
    }));
}
//# sourceMappingURL=index.js.map