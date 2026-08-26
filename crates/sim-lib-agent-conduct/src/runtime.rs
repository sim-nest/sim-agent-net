impl AgentConduct {
    fn compile(&self, cx: &mut Cx) -> Result<CompiledGraph> {
        compile_graph(cx, &self.topology.graph)
    }

    /// Runs through the topology engine and translates its public output envelope unchanged.
    pub fn run(&self, cx: &mut Cx, frame: Expr, bindings: TopologyBindings) -> Result<Expr> {
        let plan = self.compile(cx)?;
        let mut run = sim_lib_topology::run::TopologyRun::new(&self.topology.graph, &plan, frame)?;
        run.set_bindings(bindings);
        run.run(cx)?;
        Ok(run.output_expr())
    }

    /// Advances exactly one topology work item, optionally resuming a continuation.
    pub fn step(
        &self,
        cx: &mut Cx,
        frame: Expr,
        continuation: Option<TopologyContinuation>,
        bindings: TopologyBindings,
    ) -> Result<AgentConductProgress> {
        let plan = self.compile(cx)?;
        let mut run = match continuation {
            Some(saved) => sim_lib_topology::run::TopologyRun::resume(
                &self.topology.graph,
                &plan,
                saved,
                bindings,
            )?,
            None => {
                let mut run =
                    sim_lib_topology::run::TopologyRun::new(&self.topology.graph, &plan, frame)?;
                run.set_bindings(bindings);
                run
            }
        };
        let progress = run.step(cx)?;
        Ok(AgentConductProgress {
            progress,
            continuation: run.continuation(),
            output: run.output_expr(),
        })
    }

    /// Reflects graph structure through the topology reflection policy.
    pub fn reflect(&self, cx: &Cx) -> Expr {
        topology_reflect_graph(cx, &self.topology.graph)
    }

    /// Runs and reports through the topology reporting implementation.
    pub fn report(&self, cx: &mut Cx, frame: Expr) -> Result<TopologyRunReport> {
        let plan = self.compile(cx)?;
        topology_reflect(cx, &self.topology.graph, &plan, frame)
    }

    /// Returns the topology-owned canonical graph projection used by diagram clients.
    pub fn diagram(&self, cx: &Cx) -> Expr {
        topology_reflect_graph(cx, &self.topology.graph)
    }
}
