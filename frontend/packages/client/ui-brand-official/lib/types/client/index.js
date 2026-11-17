import { PixelWhale } from "./home-whale/PixelWhale.js";
import { OfficialBrandMark, OfficialBrandName } from "./Brand.js";
/** Required service: the UI slot registry. */
export const inject = ['slots'];
/**
 * Fill official sidebar artwork and frame the blank home with the whale decoration.
 * @param ctx - Client root context.
 */
export function apply(ctx) {
    ctx.slots.inject('conversation.hero.frame', () => ctx.slots.register({
        name: 'conversation.hero.frame',
        id: 'pixel-whale',
    }, PixelWhale));
    if (process.env.DSH_CLIENT_BUILD_PROFILE !== 'official')
        return;
    ctx.slots.inject('sidebar.brand.mark', () => ctx.slots.inject('sidebar.brand.name', function* () {
        yield ctx.slots.register({ name: 'sidebar.brand.mark' }, OfficialBrandMark);
        yield ctx.slots.register({ name: 'sidebar.brand.name' }, OfficialBrandName);
    }));
}
//# sourceMappingURL=index.js.map