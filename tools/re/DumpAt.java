import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionManager;

public class DumpAt extends GhidraScript {
    public void run() throws Exception {
        DecompInterface di = new DecompInterface();
        di.toggleCCode(true);
        di.toggleSyntaxTree(true);
        di.openProgram(currentProgram);
        FunctionManager fm = currentProgram.getFunctionManager();
        for (String a : getScriptArgs()) {
            Address addr = currentProgram.getAddressFactory().getDefaultAddressSpace().getAddress(a);
            Function f = fm.getFunctionContaining(addr);
            if (f == null) { println("### no function containing " + a); continue; }
            DecompileResults r = di.decompileFunction(f, 120, monitor);
            println("### FUNCTION " + f.getName() + " @ " + f.getEntryPoint());
            if (r.decompileCompleted()) println(r.getDecompiledFunction().getC());
            else println("### decompile failed: " + r.getErrorMessage());
        }
    }
}
