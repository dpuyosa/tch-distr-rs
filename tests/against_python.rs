use ndarray::{array, ArrayD};
use numpy::{PyArrayDyn, ToPyArray};
use pyo3::{prelude::*, types::PyTuple};
use serial_test::serial;
use std::convert::{TryFrom, TryInto};
use tch::Tensor;
use tch_distr::{
    Bernoulli, Cauchy, Distribution, Exponential, Gamma, Geometric, KullackLeiberDivergence,
    MultivariateNormal, Normal, Poisson, Uniform,
};

const SEED: i64 = 42;

struct PyEnv<'py> {
    py: Python<'py>,
    torch: &'py PyModule,
    distributions: &'py PyModule,
    kl: &'py PyModule,
}

impl<'py> PyEnv<'py> {
    fn new(gil: &'py GILGuard) -> Self {
        let py = gil.python();

        let torch = PyModule::import(py, "torch").unwrap();
        let distributions = PyModule::import(py, "torch.distributions").unwrap();
        let kl = PyModule::import(py, "torch.distributions.kl").unwrap();

        Self {
            py,
            torch,
            distributions,
            kl,
        }
    }
}

struct TestCases {
    entropy: bool,
    log_prob: Option<Vec<Tensor>>,
    cdf: Option<Vec<Tensor>>,
    icdf: Option<Vec<Tensor>>,
    sample: Option<Vec<Vec<i64>>>,
}

impl Default for TestCases {
    fn default() -> Self {
        Self {
            entropy: true,
            log_prob: Some(vec![
                1.0.into(),
                2.0.into(),
                Tensor::of_slice(&[1.0, 1.0]),
                Tensor::of_slice(&[2.0, 2.0]),
            ]),
            cdf: Some(vec![
                1.0.into(),
                2.0.into(),
                Tensor::of_slice(&[1.0, 1.0]),
                Tensor::of_slice(&[2.0, 2.0]),
            ]),
            icdf: Some(vec![
                0.5.into(),
                0.7.into(),
                Tensor::of_slice(&[0.3, 0.4]),
                Tensor::of_slice(&[0.2, 0.7]),
            ]),
            sample: None,
        }
    }
}

fn tensor_to_py_obj<'py>(py_env: &'py PyEnv, t: &Tensor) -> &'py PyAny {
    let array: ndarray::ArrayD<f64> = t.try_into().unwrap();
    py_env
        .torch
        .getattr("from_numpy")
        .expect("call from_numpy failed")
        .call1((array.to_pyarray(py_env.py),))
        .unwrap()
}

fn assert_tensor_eq<'py>(py: Python<'py>, t: &Tensor, py_t: &PyAny) {
    let pyarray: &PyArrayDyn<f64> = py_t.call_method0("numpy").unwrap().extract().unwrap();
    let array: ArrayD<f64> = t.try_into().unwrap();
    assert_eq!(
        array.to_pyarray(py).as_cell_slice().unwrap(),
        pyarray.as_cell_slice().unwrap()
    );
}

fn test_entropy<D: Distribution>(py_env: &PyEnv, dist_rs: &D, dist_py: &PyAny) {
    let entropy_py = dist_py.call_method0("entropy").unwrap();
    let entropy_rs = dist_rs.entropy();
    assert_tensor_eq(py_env.py, &entropy_rs, entropy_py);
}

fn test_log_prob<D: Distribution>(py_env: &PyEnv, dist_rs: &D, dist_py: &PyAny, args: &[Tensor]) {
    for args in args.iter() {
        let args_py = PyTuple::new(py_env.py, vec![tensor_to_py_obj(py_env, args)]);
        let log_prob_py = dist_py.call_method1("log_prob", args_py).unwrap();
        let log_prob_rs = dist_rs.log_prob(args);
        assert_tensor_eq(py_env.py, &log_prob_rs, log_prob_py);
    }
}

fn test_cdf<D: Distribution>(py_env: &PyEnv, dist_rs: &D, dist_py: &PyAny, args: &[Tensor]) {
    for args in args.iter() {
        let args_py = PyTuple::new(py_env.py, vec![tensor_to_py_obj(py_env, args)]);
        let log_prob_py = dist_py.call_method1("cdf", args_py).unwrap();
        let log_prob_rs = dist_rs.cdf(args);
        assert_tensor_eq(py_env.py, &log_prob_rs, log_prob_py);
    }
}

fn test_icdf<D: Distribution>(py_env: &PyEnv, dist_rs: &D, dist_py: &PyAny, args: &[Tensor]) {
    for args in args.into_iter() {
        let args_py = PyTuple::new(py_env.py, vec![tensor_to_py_obj(py_env, args)]);
        let log_prob_py = dist_py.call_method1("icdf", args_py).unwrap();
        let log_prob_rs = dist_rs.icdf(args);
        assert_tensor_eq(py_env.py, &log_prob_rs, log_prob_py);
    }
}

fn test_sample<D: Distribution>(py_env: &PyEnv, dist_rs: &D, dist_py: &PyAny, args: &[Vec<i64>]) {
    for args in args.into_iter() {
        // We need to ensure that we always start with the same seed.
        tch::manual_seed(SEED);
        let samples_py = dist_py
            .call_method1("sample", (args.to_object(py_env.py),))
            .unwrap();
        tch::manual_seed(SEED);
        let samples_rs = dist_rs.sample(args);
        assert_tensor_eq(py_env.py, &samples_rs, samples_py);
    }
}

fn test_rsample_of_normal_distribution(
    py_env: &PyEnv,
    dist_rs: &Normal,
    dist_py: &PyAny,
    args: &[Vec<i64>],
) {
    for args in args.into_iter() {
        // We need to ensure that we always start with the same seed.
        tch::manual_seed(SEED);
        let samples_py = dist_py
            .call_method1("rsample", (args.to_object(py_env.py),))
            .unwrap();
        tch::manual_seed(SEED);
        let samples_rs = dist_rs.rsample(args);
        assert_tensor_eq(py_env.py, &samples_rs, samples_py);
    }
}

fn test_kl_divergence<P, Q>(
    py_env: &PyEnv,
    dist_p_rs: &P,
    dist_q_rs: &Q,
    dist_p_py: &PyAny,
    dist_q_py: &PyAny,
) where
    P: Distribution + KullackLeiberDivergence<Q>,
    Q: Distribution,
{
    let args_py = PyTuple::new(py_env.py, vec![dist_p_py, dist_q_py]);
    let kl_divergence_py = py_env.kl.call_method1("kl_divergence", args_py).unwrap();
    let kl_divergence_rs = dist_p_rs.kl_divergence(&dist_q_rs);
    assert_tensor_eq(py_env.py, &kl_divergence_rs, kl_divergence_py);
}

fn run_test_cases<D>(py_env: &PyEnv, dist_rs: D, dist_py: &PyAny, test_cases: &TestCases)
where
    D: Distribution,
{
    if test_cases.entropy {
        test_entropy(py_env, &dist_rs, dist_py);
    }
    if let Some(log_prob) = test_cases.log_prob.as_ref() {
        test_log_prob(py_env, &dist_rs, dist_py, &log_prob);
    }
    if let Some(cdf) = test_cases.cdf.as_ref() {
        test_cdf(py_env, &dist_rs, dist_py, &cdf);
    }
    if let Some(icdf) = test_cases.icdf.as_ref() {
        test_icdf(py_env, &dist_rs, dist_py, icdf);
    }
    if let Some(sample) = test_cases.sample.as_ref() {
        test_sample(py_env, &dist_rs, dist_py, sample);
    }
}

#[test]
#[serial]
fn normal() {
    let gil = Python::acquire_gil();
    let py_env = PyEnv::new(&gil);

    let args: Vec<(Tensor, Tensor)> = vec![
        (1.0.into(), 2.0.into()),
        (2.0.into(), 4.0.into()),
        (Tensor::of_slice(&[1.0, 1.0]), Tensor::of_slice(&[2.0, 2.0])),
    ];

    let mut test_cases = TestCases::default();
    test_cases.sample = Some(vec![vec![1], vec![1, 2]]);

    for (mean, std) in args.into_iter() {
        let dist_py = py_env
            .distributions
            .getattr("Normal")
            .expect("call Normal failed")
            .call1((
                tensor_to_py_obj(&py_env, &mean),
                tensor_to_py_obj(&py_env, &std),
            ))
            .unwrap();
        let dist_rs = Normal::new(mean, std);

        // The test of rsampling is not in function `run_test_cases`,
        // because `rsample` is not a method of trait `Distribution`
        if let Some(sample) = &test_cases.sample {
            test_rsample_of_normal_distribution(&py_env, &dist_rs, dist_py, sample);
        }
        
        //genral test
        run_test_cases(&py_env, dist_rs, dist_py, &test_cases);
    }

    let p_q_mean_std: Vec<((Tensor, Tensor), (Tensor, Tensor))> =
        vec![((1.0.into(), 2.0.into()), (2.0.into(), 3.0.into()))];

    for ((p_mean, p_std), (q_mean, q_std)) in p_q_mean_std {
        let dist_p_py = py_env
            .distributions
            .getattr("Normal")
            .expect("call Normal failed")
            .call1((
                tensor_to_py_obj(&py_env, &p_mean),
                tensor_to_py_obj(&py_env, &p_std),
            ))
            .unwrap();
        let dist_p_rs = Normal::new(p_mean, p_std);

        let dist_q_py = py_env
            .distributions
            .getattr("Normal")
            .expect("call Normal failed")
            .call1((
                tensor_to_py_obj(&py_env, &q_mean),
                tensor_to_py_obj(&py_env, &q_std),
            ))
            .unwrap();
        let dist_q_rs = Normal::new(q_mean, q_std);

        test_kl_divergence(&py_env, &dist_p_rs, &dist_q_rs, dist_p_py, dist_q_py);
    }
}

#[test]
#[serial]
fn uniform() {
    let gil = Python::acquire_gil();
    let py_env = PyEnv::new(&gil);

    let args: Vec<(Tensor, Tensor)> = vec![
        (1.0.into(), 2.0.into()),
        (2.0.into(), 4.0.into()),
        (Tensor::of_slice(&[1.0, 1.0]), Tensor::of_slice(&[2.0, 2.0])),
    ];

    let mut test_cases = TestCases::default();
    test_cases.sample = Some(vec![vec![1], vec![1, 2]]);

    for (low, high) in args.into_iter() {
        let dist_py = py_env
            .distributions
            .getattr("Uniform")
            .expect("call Uniform failed")
            .call1((
                tensor_to_py_obj(&py_env, &low),
                tensor_to_py_obj(&py_env, &high),
            ))
            .unwrap();
        let dist_rs = Uniform::new(low, high);
        run_test_cases(&py_env, dist_rs, dist_py, &test_cases);
    }

    let p_q_mean_std: Vec<((Tensor, Tensor), (Tensor, Tensor))> = vec![
        ((0.0.into(), 3.0.into()), (1.0.into(), 3.0.into())),
        ((1.0.into(), 2.0.into()), (0.0.into(), 3.0.into())),
    ];

    for ((p_low, p_high), (q_low, q_high)) in p_q_mean_std {
        let dist_p_py = py_env
            .distributions
            .getattr("Uniform")
            .expect("call Uniform failed")
            .call1((
                tensor_to_py_obj(&py_env, &p_low),
                tensor_to_py_obj(&py_env, &p_high),
            ))
            .unwrap();
        let dist_p_rs = Uniform::new(p_low, p_high);

        let dist_q_py = py_env
            .distributions
            .getattr("Uniform")
            .expect("call Uniform failed")
            .call1((
                tensor_to_py_obj(&py_env, &q_low),
                tensor_to_py_obj(&py_env, &q_high),
            ))
            .unwrap();
        let dist_q_rs = Uniform::new(q_low, q_high);

        test_kl_divergence(&py_env, &dist_p_rs, &dist_q_rs, dist_p_py, dist_q_py);
    }
}

#[test]
#[serial]
fn bernoulli() {
    let gil = Python::acquire_gil();
    let py_env = PyEnv::new(&gil);

    let probs: Vec<Tensor> = vec![0.1337.into(), 0.6667.into()];

    let mut test_cases = TestCases::default();
    test_cases.icdf = None;
    test_cases.cdf = None;
    test_cases.sample = Some(vec![vec![1], vec![1, 2]]);

    for probs in probs.into_iter() {
        let dist_py = py_env
            .distributions
            .getattr("Bernoulli")
            .expect("call Bernoulli failed")
            .call1((tensor_to_py_obj(&py_env, &probs),))
            .unwrap();
        let dist_rs = Bernoulli::from_probs(probs);
        run_test_cases(&py_env, dist_rs, dist_py, &test_cases);
    }

    let logits: Vec<Tensor> = vec![0.1337.into(), 0.6667.into()];

    let mut test_cases = TestCases::default();
    test_cases.icdf = None;
    test_cases.cdf = None;
    test_cases.sample = Some(vec![vec![1], vec![1, 2]]);

    for logits in logits.into_iter() {
        let dist_py = py_env
            .distributions
            .getattr("Bernoulli")
            .expect("call Bernoulli failed")
            .call1((
                pyo3::Python::None(py_env.py),
                tensor_to_py_obj(&py_env, &logits).to_object(py_env.py),
            ))
            .unwrap();
        let dist_rs = Bernoulli::from_logits(logits);
        run_test_cases(&py_env, dist_rs, dist_py, &test_cases);
    }

    let p_q_probs: Vec<(Tensor, Tensor)> =
        vec![(0.3.into(), 0.65.into()), (0.11237.into(), 0.898.into())];

    for (p_probs, q_probs) in p_q_probs {
        let dist_p_py = py_env
            .distributions
            .getattr("Bernoulli")
            .expect("call Bernoulli failed")
            .call1((tensor_to_py_obj(&py_env, &p_probs),))
            .unwrap();
        let dist_p_rs = Bernoulli::from_probs(p_probs);

        let dist_q_py = py_env
            .distributions
            .getattr("Bernoulli")
            .expect("call Bernoulli failed")
            .call1((tensor_to_py_obj(&py_env, &q_probs),))
            .unwrap();
        let dist_q_rs = Bernoulli::from_probs(q_probs);

        test_kl_divergence(&py_env, &dist_p_rs, &dist_q_rs, dist_p_py, dist_q_py);
    }
}

#[test]
#[serial]
fn poisson() {
    let gil = Python::acquire_gil();
    let py_env = PyEnv::new(&gil);

    let rates: Vec<Tensor> = vec![
        0.1337.into(),
        0.6667.into(),
        Tensor::of_slice(&[0.156, 0.33]),
    ];

    let mut test_cases = TestCases::default();
    test_cases.cdf = None;
    test_cases.icdf = None;
    test_cases.entropy = false;
    test_cases.sample = Some(vec![vec![1], vec![1, 2]]);

    for rate in rates.into_iter() {
        let dist_py = py_env
            .distributions
            .getattr("Poisson")
            .expect("call Poisson failed")
            .call1((tensor_to_py_obj(&py_env, &rate),))
            // .call1("Poisson", (tensor_to_py_obj(&py_env, &rate),))
            .unwrap();
        let dist_rs = Poisson::new(rate);
        run_test_cases(&py_env, dist_rs, dist_py, &test_cases);
    }

    let p_q_rate: Vec<(Tensor, Tensor)> = vec![(1.0.into(), 2.0.into()), (3.0.into(), 4.0.into())];

    for (p_rate, q_rate) in p_q_rate {
        let dist_p_py = py_env
            .distributions
            .getattr("Poisson")
            .expect("call Poisson failed")
            .call1((tensor_to_py_obj(&py_env, &p_rate),))
            .unwrap();
        let dist_p_rs = Poisson::new(p_rate);

        let dist_q_py = py_env
            .distributions
            .getattr("Poisson")
            .expect("call Poisson failed")
            .call1((tensor_to_py_obj(&py_env, &q_rate),))
            .unwrap();
        let dist_q_rs = Poisson::new(q_rate);

        test_kl_divergence(&py_env, &dist_p_rs, &dist_q_rs, dist_p_py, dist_q_py);
    }
}

#[test]
#[serial]
fn exponential() {
    let gil = Python::acquire_gil();
    let py_env = PyEnv::new(&gil);

    let rates: Vec<Tensor> = vec![
        0.1337.into(),
        0.6667.into(),
        Tensor::of_slice(&[0.156, 0.33]),
    ];

    let mut test_cases = TestCases::default();
    test_cases.sample = Some(vec![vec![1], vec![1, 2]]);

    for rate in rates.into_iter() {
        let dist_py = py_env
            .distributions
            .getattr("Exponential")
            .expect("call Exponential failed")
            .call1((tensor_to_py_obj(&py_env, &rate),))
            .unwrap();
        let dist_rs = Exponential::new(rate);
        run_test_cases(&py_env, dist_rs, dist_py, &test_cases);
    }

    let p_q_rate: Vec<(Tensor, Tensor)> = vec![(0.3.into(), 0.7.into()), (0.6.into(), 0.5.into())];

    for (p_rate, q_rate) in p_q_rate {
        let dist_p_py = py_env
            .distributions
            .getattr("Exponential")
            .expect("call Exponential failed")
            .call1((tensor_to_py_obj(&py_env, &p_rate),))
            .unwrap();
        let dist_p_rs = Exponential::new(p_rate);

        let dist_q_py = py_env
            .distributions
            .getattr("Exponential")
            .expect("call Exponential failed")
            .call1((tensor_to_py_obj(&py_env, &q_rate),))
            .unwrap();
        let dist_q_rs = Exponential::new(q_rate);

        test_kl_divergence(&py_env, &dist_p_rs, &dist_q_rs, dist_p_py, dist_q_py);
    }
}

#[test]
#[serial]
fn cauchy() {
    let gil = Python::acquire_gil();
    let py_env = PyEnv::new(&gil);

    let args: Vec<(Tensor, Tensor)> = vec![
        (1.0.into(), 2.0.into()),
        (2.0.into(), 4.0.into()),
        (Tensor::of_slice(&[1.0, 1.0]), Tensor::of_slice(&[2.0, 2.0])),
    ];

    let mut test_cases = TestCases::default();
    test_cases.sample = Some(vec![vec![1], vec![1, 2]]);

    for (median, scale) in args.into_iter() {
        let dist_py = py_env
            .distributions
            .getattr("Cauchy")
            .expect("call Cauchy failed")
            .call1((
                tensor_to_py_obj(&py_env, &median),
                tensor_to_py_obj(&py_env, &scale),
            ))
            .unwrap();
        let dist_rs = Cauchy::new(median, scale);
        run_test_cases(&py_env, dist_rs, dist_py, &test_cases);
    }
}

#[test]
#[serial]
fn gamma() {
    let gil = Python::acquire_gil();
    let py_env = PyEnv::new(&gil);

    let args: Vec<(Tensor, Tensor)> = vec![
        (1.0.into(), 2.0.into()),
        (2.0.into(), 4.0.into()),
        (Tensor::of_slice(&[1.0, 1.0]), Tensor::of_slice(&[2.0, 2.0])),
    ];

    let mut test_cases = TestCases::default();
    test_cases.cdf = None;
    test_cases.icdf = None;

    for (concentration, rate) in args.into_iter() {
        let dist_py = py_env
            .distributions
            .getattr("Gamma")
            .expect("call Gamma failed")
            .call1((
                tensor_to_py_obj(&py_env, &concentration),
                tensor_to_py_obj(&py_env, &rate),
            ))
            .unwrap();
        let dist_rs = Gamma::new(concentration, rate);
        run_test_cases(&py_env, dist_rs, dist_py, &test_cases);
    }

    let p_q_concentration_rate: Vec<((Tensor, Tensor), (Tensor, Tensor))> =
        vec![((0.3.into(), 0.7.into()), (0.6.into(), 0.5.into()))];

    for ((p_concentration, p_rate), (q_concentration, q_rate)) in p_q_concentration_rate {
        let dist_p_py = py_env
            .distributions
            .getattr("Gamma")
            .expect("call Gamma failed")
            .call1((
                tensor_to_py_obj(&py_env, &p_concentration),
                tensor_to_py_obj(&py_env, &p_rate),
            ))
            .unwrap();
        let dist_p_rs = Gamma::new(p_concentration, p_rate);

        let dist_q_py = py_env
            .distributions
            .getattr("Gamma")
            .expect("call Gamma failed")
            .call1((
                tensor_to_py_obj(&py_env, &q_concentration),
                tensor_to_py_obj(&py_env, &q_rate),
            ))
            .unwrap();
        let dist_q_rs = Gamma::new(q_concentration, q_rate);

        test_kl_divergence(&py_env, &dist_p_rs, &dist_q_rs, dist_p_py, dist_q_py);
    }
}

#[test]
#[serial]
fn geometric() {
    let gil = Python::acquire_gil();
    let py_env = PyEnv::new(&gil);

    let probs: Vec<Tensor> = vec![0.1337.into(), 0.6667.into(), 1.0.into()];

    let mut test_cases = TestCases::default();
    test_cases.icdf = None;
    test_cases.cdf = None;
    test_cases.sample = Some(vec![vec![1], vec![1, 2]]);

    for probs in probs.into_iter() {
        let dist_py = py_env
            .distributions
            .getattr("Geometric")
            .expect("call Geometric failed")
            .call1((tensor_to_py_obj(&py_env, &probs),))
            .unwrap();
        let dist_rs = Geometric::from_probs(probs);
        run_test_cases(&py_env, dist_rs, dist_py, &test_cases);
    }

    let logits: Vec<Tensor> = vec![0.1337.into(), 0.6667.into(), 1.0.into()];

    let mut test_cases = TestCases::default();
    test_cases.icdf = None;
    test_cases.cdf = None;

    for logits in logits.into_iter() {
        let dist_py = py_env
            .distributions
            .getattr("Geometric")
            .expect("call Geometric failed")
            .call1((
                pyo3::Python::None(py_env.py),
                tensor_to_py_obj(&py_env, &logits).to_object(py_env.py),
            ))
            .unwrap();
        let dist_rs = Geometric::from_logits(logits);
        run_test_cases(&py_env, dist_rs, dist_py, &test_cases);
    }

    let p_q_probs: Vec<(Tensor, Tensor)> = vec![(0.3.into(), 0.7.into()), (0.6.into(), 0.5.into())];

    for (p_probs, q_probs) in p_q_probs {
        let dist_p_py = py_env
            .distributions
            .getattr("Geometric")
            .expect("call Geometric failed")
            .call1((tensor_to_py_obj(&py_env, &p_probs),))
            .unwrap();
        let dist_p_rs = Geometric::from_probs(p_probs);

        let dist_q_py = py_env
            .distributions
            .getattr("Geometric")
            .expect("call Geometric failed")
            .call1((tensor_to_py_obj(&py_env, &q_probs),))
            .unwrap();
        let dist_q_rs = Geometric::from_probs(q_probs);

        test_kl_divergence(&py_env, &dist_p_rs, &dist_q_rs, dist_p_py, dist_q_py);
    }
}

#[test]
#[serial]
fn multivariate_normal() {
    let gil = Python::acquire_gil();
    let py_env = PyEnv::new(&gil);

    let mean_and_covs: Vec<(Tensor, Tensor)> = vec![
        (
            Tensor::of_slice(&[1.0]),
            Tensor::try_from(array![[1.0, 0.0], [0.0, 1.0]]).unwrap(),
        ),
        (
            Tensor::of_slice(&[1.0, 2.0, 3.0]),
            Tensor::try_from(array![[3.0, 0.0, 0.0], [0.0, 7.0, 0.0], [0.0, 0.0, 10.0]]).unwrap(),
        ),
    ];

    let mut test_cases = TestCases::default();
    test_cases.icdf = None;
    test_cases.cdf = None;
    test_cases.entropy = false;
    //test_cases.sample = Some(vec![vec![1], vec![1, 2]]);
    test_cases.sample = Some(vec![vec![1]]);
    test_cases.log_prob = None;

    for (mean, cov) in mean_and_covs.into_iter() {
        let dist_py = py_env
            .distributions
            .getattr("MultivariateNormal")
            .expect("call MultivariateNormal failed")
            .call1((
                tensor_to_py_obj(&py_env, &mean),
                tensor_to_py_obj(&py_env, &cov),
            ))
            .unwrap();
        let dist_rs = MultivariateNormal::from_cov(mean, cov);
        run_test_cases(&py_env, dist_rs, dist_py, &test_cases);
    }

    let mean_and_precisions: Vec<(Tensor, Tensor)> = vec![(
        Tensor::of_slice(&[1.0]),
        Tensor::try_from(array![[1.0, 0.0], [0.0, 1.0]]).unwrap(),
    )];

    let mut test_cases = TestCases::default();
    test_cases.icdf = None;
    test_cases.cdf = None;
    test_cases.sample = Some(vec![vec![1], vec![1, 2]]);

    for (mean, precision) in mean_and_precisions.into_iter() {
        let dist_py = py_env
            .distributions
            .getattr("MultivariateNormal")
            .expect("call MultivariateNormal failed")
            .call1((
                tensor_to_py_obj(&py_env, &mean),
                pyo3::Python::None(py_env.py),
                tensor_to_py_obj(&py_env, &precision),
            ))
            .unwrap();
        let dist_rs = MultivariateNormal::from_precision(mean, precision);
        run_test_cases(&py_env, dist_rs, dist_py, &test_cases);
    }

    let mean_and_scale_trils: Vec<(Tensor, Tensor)> = vec![(
        Tensor::of_slice(&[1.0]),
        Tensor::try_from(array![[1.0, 0.0], [0.0, 1.0]]).unwrap(),
    )];

    let mut test_cases = TestCases::default();
    test_cases.icdf = None;
    test_cases.cdf = None;
    test_cases.sample = Some(vec![vec![1], vec![1, 2]]);

    for (mean, scale_tril) in mean_and_scale_trils.into_iter() {
        let dist_py = py_env
            .distributions
            .getattr("MultivariateNormal")
            .expect("call MultivariateNormal failed")
            .call1((
                tensor_to_py_obj(&py_env, &mean),
                pyo3::Python::None(py_env.py),
                pyo3::Python::None(py_env.py),
                tensor_to_py_obj(&py_env, &scale_tril),
            ))
            .unwrap();
        let dist_rs = MultivariateNormal::from_scale_tril(mean, scale_tril);
        run_test_cases(&py_env, dist_rs, dist_py, &test_cases);
    }
}
