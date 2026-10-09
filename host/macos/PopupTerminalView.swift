import AppKit

/// SwiftTerm draws iTerm2 and sixel pictures in points but answers the cell-size query in backing
/// pixels, so on a Retina display a program that sizes its pictures from the answer (ratatui-image)
/// draws them twice too big. This view answers in points, which is what its pictures are measured in.
final class PopupTerminalView: LocalProcessTerminalView {
    override func windowCommand(source: Terminal, command: Terminal.WindowManipulationCommand) -> [UInt8]? {
        guard case .reportCellSizeInPixels = command, let cell = cellDimension else {
            return super.windowCommand(source: source, command: command)
        }
        return source.cc.CSI + "6;\(Int(cell.height.rounded()));\(Int(cell.width.rounded()))t".utf8
    }
}
