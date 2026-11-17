/**
 * Canonical Office tool results and their model-facing content.
 * @module @deepseek-ai/dsh-tool-office/content
 */
import { AttachmentId } from '@deepseek-ai/dsh-attachment';
function fontsLine(missingFonts) {
    return missingFonts.length === 0 ? '' : `\nFonts not installed and substituted: ${missingFonts.join(', ')}.`;
}
function location(image) {
    if (image.page !== undefined)
        return `Page ${image.page}`;
    if (image.sheet === undefined)
        return 'Image';
    return image.range === undefined ? `Sheet "${image.sheet}"` : `Sheet "${image.sheet}" ${image.range}`;
}
/**
 * Project a render into a summary, then one label and image per rendered page or area.
 * @param value - canonical render result.
 * @returns text and image blocks in document order.
 */
export function renderImagesContent(value) {
    const count = value.images.length;
    const blocks = [{
            type: 'text',
            text: `Rendered ${count} image${count === 1 ? '' : 's'} of ${value.path} (${value.pageCount} in the document) at ${value.dpi} DPI.${fontsLine(value.missingFonts)}`,
        }];
    for (const image of value.images) {
        blocks.push({ type: 'text', text: `${location(image)}: ${image.width}x${image.height} px` });
        blocks.push({ type: 'image', attachment: {
                attachmentId: AttachmentId(image.attachmentId), mediaType: image.mediaType, bytes: image.bytes,
                width: image.width, height: image.height,
                ...image.originalDimensions === undefined ? {} : { originalDimensions: { ...image.originalDimensions } },
            } });
    }
    return blocks;
}
/**
 * Project a conversion or recalculation into its temporary file and the copy instruction.
 * @param value - canonical file result.
 * @returns one text block naming the temporary file.
 */
export function renderFileContent(value) {
    const verb = value.operation === 'convert' ? 'Converted' : 'Recalculated';
    const pages = value.pageCount === undefined ? '' : `, ${value.pageCount} page${value.pageCount === 1 ? '' : 's'}`;
    return [{ type: 'text', text: `${verb} ${value.path} to ${value.format} (${value.bytes} bytes${pages}): ${value.file}\n`
                + `Copy this temporary file to its destination with the shell now; it may be deleted after a retention period or to free space, and is deleted when this conversation ends.${fontsLine(value.missingFonts)}` }];
}
//# sourceMappingURL=content.js.map